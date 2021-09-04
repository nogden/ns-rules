#![feature(iter_intersperse)]

use std::{
    process, fmt, fs, iter,
    ffi::OsStr,
    path::{self, Path, PathBuf},
    str::FromStr
};
use regex::Regex;
use walkdir::WalkDir;
use thiserror::Error;
use miette::{
    Diagnostic,
    DiagnosticResult,
    DiagnosticReportPrinter,
    GraphicalReportPrinter,
    NamedSource,
    SourceSpan
};
use owo_colors::OwoColorize;
use clap::{AppSettings, Clap};

mod config;

/// Applies namespace referencing rules to Clojure source code.
#[derive(Clap)]
#[clap(version = "1.0", author = "Nick Ogden <nick@nickogden.org>")]
#[clap(setting = AppSettings::ColoredHelp)]
pub(crate) struct Options {
    /// The path to the configuration file.
    #[clap(short, long, default_value = "ns-rules.edn")]
    config: PathBuf,

    /// The number of lines of context to print around each violation.
    #[clap(short = 'n', long, default_value = "4")]
    context_lines: usize,
}

fn main() -> DiagnosticResult<()> {
    let options = Options::parse();
    let config = config::read_file(options.config)?;

    // build rule set
    let rules = vec![Rule {
        namespace: "duka.boundary.*".parse().unwrap(),
        allow: vec![
            "duka.boundary.*".parse().unwrap(),
            "duka.domain.*".parse().unwrap(),
            "duka.db.*".parse().unwrap()
        ]
    }];

    let mut report = Report::new();
    let source_files = find_source_files(&config.source_dirs, &mut report);

    // compile rules against available namespaces
    let compiled_rules: Vec<_> = rules.into_iter()
        .map(|rule| rule.compile(&source_files))
        .collect();

    apply_rules(&compiled_rules, &source_files, &mut report);

    print!("{}", report);
    process::exit(report.exit_status());
}

fn find_source_files<P: AsRef<Path> + std::fmt::Debug> (
    source_dirs: &[P], report: &mut Report,
) -> Vec<ClojureSourceFile> {
    let mut source_files = Vec::new();
    for source_dir in source_dirs {
        let source_tree = WalkDir::new(&source_dir).min_depth(1);
        for entry in source_tree {
            let file = match entry {
                Ok(entry) if entry.file_type().is_file() => entry,
                Err(error) => {
                    report.file_skipped(error.to_string());
                    continue;
                }
                _ => continue // skip non-files
            };

            let ext = file.path().extension().and_then(OsStr::to_str);
            if let Some("clj" | "cljs" | "cljc") = ext {   //  v---- source_dir
                let ns = file.path()            // ~/dev/proj/src/com/my_org/core.clj
                    .strip_prefix(&source_dir)             //     com/my_org/core.clj
                    .expect("source root was not a prefix of file path")
                    .as_os_str()
                    .to_str()
                    .and_then(|path| {
                        let ns = path.rsplit_once('.')     //     (com/my_org/core|clj)
                            .expect("file path with clojure extension did not contain '.'")
                            .0                             //      com/my_org/core
                            .replace(path::MAIN_SEPARATOR, ".") // com.my_org.core
                            .replace('_', "-");            //      com.my-org.core
                        Some(ns)
                    });

                let path = file.path().as_os_str().to_str();
                if let (Some(mut ns), Some(path)) = (ns, path) {
                    let path_start = ns.len();
                    ns.push_str(path);
                    source_files.push(ClojureSourceFile { entry: ns, path_start });
                } else {
                    report.file_skipped(format!(
                        "path {} contains invalid utf8 characters, skipping",
                        &file.path().display()
                    ));
                }
            } else /* not a Clojure source file */ {
                report.file_skipped(format!(
                    "{} is not a Clojure source file, skipping",
                    file.path().display()
                ));
            }
        }

    }
    report.candidate_files(&source_files);

    source_files
}

#[derive(Debug)]
struct ClojureSourceFile{
    entry: String,
    path_start: usize,
}

impl ClojureSourceFile {
    fn path(&self) -> &str {
        &self.entry[self.path_start..]
    }

    fn namespace(&self) -> &str {
        &self.entry[..self.path_start]
    }
}

fn apply_rules(
    rules: &[CompiledRule], source_files: &[ClojureSourceFile], report: &mut Report
) {
    for file in source_files {
        for rule in rules {
            if rule.matches(file.namespace()) {
                report.rule_matched();
                match fs::read_to_string(file.path()) {
                    Ok(code) => rule.apply(file, code, report),
                    Err(error) => {
                        report.file_skipped(
                            format!("failed to read file {}: {}", file.path(), error)
                        );
                    }
                }
                break;
            }
        }
    }
}

#[derive(Debug)]
struct Report {
    violations: Vec<Violation>,
    warnings: Vec<String>,
    namespaces_checked: usize,
    rules_matched: usize,
    files_skipped: usize,
}

impl Report {
    fn new() -> Self {
        Self {
            violations: vec![],
            warnings: vec![],
            namespaces_checked: 0,
            rules_matched: 0,
            files_skipped: 0,
        }
    }

    fn candidate_files(&mut self, files: &[ClojureSourceFile]) {
        self.namespaces_checked = files.len();
    }

    fn file_skipped(&mut self, warning: String) {
        self.warnings.push(warning);
    }

    fn violation(&mut self, violation: Violation) {
        self.violations.push(violation);
    }

    fn rule_matched(&mut self) {
        self.rules_matched += 1;
    }

    fn exit_status(&self) -> i32 {
        if self.violations.is_empty() { 0 } else { 1 }
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.warnings.is_empty() {
            f.write_str("Warnings:\n")?;
            for warning in self.warnings.iter() {
                writeln!(f, "  {}", warning)?;
            }
            f.write_str("\n")?;
        }

        let printer = GraphicalReportPrinter::new();
        for violation in self.violations.iter() {
            printer.debug(violation, f)?;
            f.write_str("\n\n")?;
        }

        if self.violations.is_empty() {
            writeln!(f, "{}", "All checks passed".green())?;
        } else {
            writeln!(
                f,
                "{}",
                format!(
                    "Found {} rule violation{}",
                    self.violations.len(),
                    self.violations.len().pluralise()
                ).red()
            )?;
        }
        writeln!(
            f,
            "{:3} namespace{} checked\n\
             {:3} rule{} matched\n\
             {:3} file{} skipped\n",
            self.namespaces_checked,
            self.namespaces_checked.pluralise(),
            self.rules_matched,
            self.rules_matched.pluralise(),
            self.warnings.len(),
            self.warnings.len().pluralise(),
        )?;

        Ok(())
    }
}

#[derive(Debug, Error, Diagnostic)]
#[error("'{src_ns}' is not permitted to reference '{ref_ns}'")]
#[diagnostic(code(namespace_rule_violation))]
struct Violation {
    src: NamedSource,
    src_ns: String,
    ref_ns: String,

    #[snippet(src, message("{}", self.src_ns.fg_rgb::<255, 135, 162>()))]
    snippet: SourceSpan,

    #[highlight(snippet, label("this reference is not allowed"))]
    ref_location: SourceSpan,
}

trait Pluralise {
    fn pluralise(&self) -> &str;
}

impl Pluralise for usize {
    fn pluralise(&self) -> &str {
        if *self == 1 { "" } else { "s" }
    }
}

#[derive(Debug)]
struct NamespaceMatcher(Regex);

impl NamespaceMatcher {
    fn matches(&self, namespace: &str) -> bool {
        self.0.is_match(namespace)
    }
}

#[derive(Debug)]
struct Rule {
    namespace: NamespaceMatcher,
    allow: Vec<NamespaceMatcher>,
    //deny: Vec<NamespaceMatcher>,
}

impl Rule {
    fn compile<'s>(self, source_files: &[ClojureSourceFile]) -> CompiledRule {
        let not_allowed = |source_file: &&ClojureSourceFile| {
            // A reference is only allowed if it is matched by an allow clause
            !self.allow.iter().any(|ns| ns.matches(source_file.namespace()))
        };

        let regex = source_files.iter()
            .filter(not_allowed)
            .map(ClojureSourceFile::namespace)
            .intersperse("|")
            .collect::<String>()
            .replace('.', "\\.");

        CompiledRule {
            namespace: self.namespace,
            checker: Regex::new(&regex).expect("compiled to invalid regex"),
        }
    }
}

#[derive(Debug)]
struct CompiledRule {
    namespace: NamespaceMatcher,
    checker: Regex,
}

impl CompiledRule {
    fn matches(&self, namespace: &str) -> bool {
        self.namespace.matches(namespace)
    }

    fn apply(&self, file: &ClojureSourceFile, code: String, report: &mut Report) {
        for reference in self.checker.find_iter(&code) {
            let ref_ns = code[reference.start()..reference.end()].to_owned();
            let snippet_start = code[..reference.start()]
                .rmatch_indices('\n')
                .nth(4)
                .map(|(i, _)| i + 1)  // Skip over the \n itself
                .unwrap_or(0);
            let snippet_end = code[reference.end()..]
                .match_indices('\n')
                .nth(4)
                .map(|(i, _)| i + reference.end())
                .unwrap_or(code.len());

            report.violation(Violation {
                src: NamedSource::new(file.path(), code.clone()),
                src_ns: file.namespace().to_owned(),
                ref_ns,
                snippet: (snippet_start, snippet_end - snippet_start).into(),
                ref_location: (
                    reference.start(), reference.end() - reference.start(),
                ).into(),
            });
        }
    }
}

impl FromStr for NamespaceMatcher {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() || s.contains(' ') || s.starts_with('.') || s.ends_with('.') {
            return Err("invalid namespace name")
        }

        // Characters allowed in EDN symbols
        // For a segment we exclude '.', but we include it for the whole ns.
        const NS_REGEX: &str = r"[[[:alnum:]]\.\*\+!\-_\?\$%\&=<>]+";
        const NS_SEGMENT_REGEX: &str = r"[[[:alnum:]]\*\+!\-_\?\$%\&=<>]+";

        let pattern: String = if let Some((head, "*")) = s.rsplit_once('.') {
            // Last element is a wildcard, os we end with recursive search
            head.split('.')
                .map(|segment| segment.replace('*', NS_SEGMENT_REGEX))
                .chain(iter::once(NS_REGEX.to_string()))
                .intersperse("\\.".to_string())
                .collect()
        } else {
            s.split('.')
                .map(|segment| segment.replace('*', NS_SEGMENT_REGEX))
                .intersperse("\\.".to_string())
                .collect()
        };

        Ok(Self(Regex::new(&pattern).expect("generated invalid regex")))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn can_match_full_namespace() {
        let matcher: NamespaceMatcher = "duka.marketplace.db".parse()
            .expect("no matcher this time :(");

        assert!(matcher.matches("duka.marketplace.db"));
        assert!(!matcher.matches("duka.marketplace.kafka"));
    }

    #[test]
    fn can_match_wildcard_within_namespace() {
        let matcher: NamespaceMatcher = "duka.market*.db".parse()
            .expect("no matcher this time :(");

        assert!(matcher.matches("duka.marketplace.db"));
        assert!(matcher.matches("duka.marketvalue.db"));
        assert!(!matcher.matches("duka.market.db"));
        assert!(!matcher.matches("duka.marketplace.kafka"));
    }

    #[test]
    fn can_match_wildcard_sub_namespace() {
        let matcher: NamespaceMatcher = "duka.marketplace.*".parse()
            .expect("no matcher this time :(");

        assert!(matcher.matches("duka.marketplace.db"));
        assert!(matcher.matches("duka.marketplace.kafka"));
        assert!(matcher.matches("duka.marketplace.db.core"));
        assert!(!matcher.matches("duka.marketplace"));
        assert!(!matcher.matches("duka.market.db"));
        assert!(!matcher.matches("karibu.marketplace.db"));

    }

    #[test]
    fn reports_error_on_invalid_namespace() {
        assert!("duka.market place.db".parse::<NamespaceMatcher>().is_err());
        assert!("".parse::<NamespaceMatcher>().is_err());
        assert!(".".parse::<NamespaceMatcher>().is_err());
        assert!(".marketplace".parse::<NamespaceMatcher>().is_err());
        assert!("marketplace.".parse::<NamespaceMatcher>().is_err());
    }
}
