#![feature(iter_intersperse)]

use clap::{AppSettings, Clap};
use miette::{
    Diagnostic, DiagnosticReportPrinter, DiagnosticResult,
    GraphicalReportPrinter, NamedSource, SourceSpan,
};
use owo_colors::OwoColorize;
use regex::Regex;
use std::{
    ffi::OsStr,
    fmt, fs, iter,
    path::{self, Path, PathBuf},
    process,
    str::FromStr,
};
use thiserror::Error;
use walkdir::WalkDir;

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
    let mut report = Report::new();

    let options = Options::parse();
    let config = config::read_file(options.config, &mut report)?;

    let source_files = find_source_files(&config.source_dirs, &mut report);

    let compiled_rules: Vec<_> = config
        .rules
        .into_iter()
        .map(|rule| rule.compile(&source_files))
        .collect();

    apply_rules(&compiled_rules, &source_files, &mut report);

    print!("{}", report);
    process::exit(report.exit_status());
}

fn find_source_files<P: AsRef<Path> + std::fmt::Debug>(
    source_dirs: &[P],
    report: &mut Report,
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
                _ => continue, // skip non-files
            };

            let ext = file.path().extension().and_then(OsStr::to_str);
            if let Some("clj" | "cljs" | "cljc") = ext {
                //  v---- source_dir
                let ns = file.path()            // ~/dev/proj/src/com/my_org/core.clj
                    .strip_prefix(&source_dir)             //     com/my_org/core.clj
                    .expect("source root is a prefix of file path")
                    .as_os_str()
                    .to_str()
                    .and_then(|path| {
                        let ns = path.rsplit_once('.')     //     (com/my_org/core|clj)
                            .expect("file path with clojure extension must contain '.'")
                            .0                             //      com/my_org/core
                            .replace(path::MAIN_SEPARATOR, ".") // com.my_org.core
                            .replace('_', "-");            //      com.my-org.core
                        Some(ns)
                    });

                let path = file.path().as_os_str().to_str();
                if let (Some(mut ns), Some(path)) = (ns, path) {
                    let path_start = ns.len();
                    ns.push_str(path);
                    source_files.push(ClojureSourceFile {
                        entry: ns,
                        path_start,
                    });
                } else {
                    report.file_skipped(format!(
                        "path {} contains invalid utf8 characters, skipping",
                        &file.path().display()
                    ));
                }
            } else
            /* not a Clojure source file */
            {
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
struct ClojureSourceFile {
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
    rules: &[CompiledRule],
    source_files: &[ClojureSourceFile],
    report: &mut Report,
) {
    for file in source_files {
        for rule in rules {
            if rule.matches(file.namespace()) {
                report.rule_matched();
                match fs::read_to_string(file.path()) {
                    Ok(code) => rule.apply(file, code, report),
                    Err(error) => {
                        report.file_skipped(format!(
                            "failed to read file {}: {}",
                            file.path(),
                            error
                        ));
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
    files_checked: usize,
    rules_matched: usize,
    files_skipped: usize,
}

impl Report {
    fn new() -> Self {
        Self {
            violations: vec![],
            warnings: vec![],
            files_checked: 0,
            rules_matched: 0,
            files_skipped: 0,
        }
    }

    fn candidate_files(&mut self, files: &[ClojureSourceFile]) {
        self.files_checked = files.len();
    }

    fn file_skipped(&mut self, warning: String) {
        self.warnings.push(warning);
        self.files_skipped += 1;
    }

    fn violation(&mut self, violation: Violation) {
        self.violations.push(violation);
    }

    fn rule_matched(&mut self) {
        self.rules_matched += 1;
    }

    fn warn(&mut self, warning: String) {
        self.warnings.push(warning);
    }

    fn exit_status(&self) -> i32 {
        if self.violations.is_empty() {
            0
        } else {
            1
        }
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
                )
                .red()
            )?;
        }
        writeln!(
            f,
            "{:3} file{} checked\n\
             {:3} namespace{} matched a rule\n\
             {:3} warning{}\n\
             {:3} file{} skipped\n",
            self.files_checked,
            self.files_checked.pluralise(),
            self.rules_matched,
            self.rules_matched.pluralise(),
            self.warnings.len(),
            self.warnings.len().pluralise(),
            self.files_skipped,
            self.files_skipped.pluralise(),
        )?;

        Ok(())
    }
}

#[derive(Debug, Error, Diagnostic)]
#[error("'{src_ns}' is not allowed to reference '{ref_ns}'")]
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
        if *self == 1 {
            ""
        } else {
            "s"
        }
    }
}

#[derive(Debug)]
struct NamespaceMatcher(Regex);

impl NamespaceMatcher {
    fn matches(&self, namespace: &str) -> bool {
        self.0.is_match(namespace)
    }
}

impl FromStr for NamespaceMatcher {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "" => Err("namespace patterns cannot be empty")?,
            s if s.contains(' ') => {
                Err("namespace patterns cannot contains spaces")?
            }
            s if s.starts_with('.') || s.ends_with('.') => {
                Err("namespace patterns cannot start with or end with '.'")?
            }
            _ => {}
        }

        // Characters allowed in EDN symbols
        // For a segment we exclude '.', but we include it for the whole ns.
        const NS_REGEX: &str = r"[[[:alnum:]]\.\*\+!\-_\?\$%\&=<>]+";
        const NS_SEGMENT_REGEX: &str = r"[[[:alnum:]]\*\+!\-_\?\$%\&=<>]+";

        let pattern: String = if let Some((head, "*")) = s.rsplit_once('.') {
            // Last element is a wildcard, so we end with recursive search
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

        Ok(Self(Regex::new(&pattern).expect("valid regex")))
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
            // Only self-references and references matched by an allow clause
            // are allowed
            let in_allow_list = self
                .allow
                .iter()
                .any(|ns| ns.matches(source_file.namespace()));
            let self_reference =
                self.namespace.matches(source_file.namespace());

            !in_allow_list && !self_reference
        };

        let regex = source_files
            .iter()
            .filter(not_allowed)
            .map(ClojureSourceFile::namespace)
            .intersperse("|")
            .collect::<String>()
            .replace('.', "\\.");

        CompiledRule {
            namespace: self.namespace,
            checker: Regex::new(&regex).expect("valid regex"),
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

    fn apply(
        &self,
        file: &ClojureSourceFile,
        code: String,
        report: &mut Report,
    ) {
        for reference in self.checker.find_iter(&code) {
            let ref_ns = code[reference.start()..reference.end()].to_owned();
            let snippet_start = code[..reference.start()]
                .rmatch_indices('\n')
                .nth(4)
                .map(|(i, _)| i + 1) // Skip over the \n itself
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
                    reference.start(),
                    reference.end() - reference.start(),
                )
                    .into(),
            });
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn can_match_full_namespace() {
        let matcher: NamespaceMatcher = "shipping.domain.ship".parse().unwrap();

        assert!(matcher.matches("shipping.domain.ship"));
        assert!(!matcher.matches("shipping.domain.port"));
    }

    #[test]
    fn can_match_wildcard_within_namespace() {
        let matcher: NamespaceMatcher = "shipping.dom*.ship".parse().unwrap();

        assert!(matcher.matches("shipping.domain.ship"));
        assert!(matcher.matches("shipping.domestic.ship"));
        assert!(!matcher.matches("shipping.use-case.routing"));
        assert!(!matcher.matches("shipping.domain.port"));
    }

    #[test]
    fn can_match_wildcard_sub_namespace() {
        let matcher: NamespaceMatcher = "shipping.use-case.*".parse().unwrap();

        assert!(matcher.matches("shipping.use-case.routing"));
        assert!(matcher.matches("shipping.use-case.contract-verification"));
        assert!(matcher.matches("shipping.use-case.routing.route"));
        assert!(!matcher.matches("shipping.use-case"));
        assert!(!matcher.matches("shipping.domain.ship"));
        assert!(!matcher.matches("flying.use-case.routing"));
    }

    #[test]
    fn reports_error_on_invalid_namespace() {
        assert!("shipping.use case.routing"
            .parse::<NamespaceMatcher>()
            .is_err());
        assert!("".parse::<NamespaceMatcher>().is_err());
        assert!(".".parse::<NamespaceMatcher>().is_err());
        assert!(".use-case".parse::<NamespaceMatcher>().is_err());
        assert!("use-case.".parse::<NamespaceMatcher>().is_err());
    }
}
