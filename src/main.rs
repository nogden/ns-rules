#![feature(iter_intersperse)]

use std::{env, ffi::OsStr, fmt, fs, iter, path::{self, Path}, str::FromStr};
use regex::Regex;
use walkdir::WalkDir;
use thiserror::Error;
use miette::{Diagnostic, DiagnosticReportPrinter, GraphicalReportPrinter, NamedSource, SourceSpan};
use owo_colors::OwoColorize;

fn main() {
    // process args
    let source_dir = env::args().skip(1).next().unwrap_or("src".to_string());

    // read config file
    // build rule set
    let rules = vec![Rule {
        namespace: "duka.boundary.*".parse().unwrap(),
        allow: vec![
            "duka.boundary.*".parse().unwrap(),
            "duka.domain.*".parse().unwrap()
        ]
    }];

    // scan for clj cljc cljs files
    // determine namespace of each file
    let (source_files, warnings) = find_source_files(&source_dir);

    // compile rules against available namespaces
    let compiled_rules: Vec<_> = rules.into_iter()
        .map(|rule| rule.compile(&source_files))
        .collect();

    // match namespace to rules
    // scan file for includes:
    //  (:require [duka.fulfillment.db])
    //  (require 'duka.fulfillment.db)
    //  (:use [duka.fulfillment.db])
    //  (use 'duka.fulfillment.db)
    //  duka.fulfillment.db/fetch-things
    // determine locations of rule violations
    let report = apply_rules(&compiled_rules, &source_files);

    // print rule violations
    print!("{}", report);
}

fn find_source_files<P: AsRef<Path> + std::fmt::Debug> (
    source_dir: P
) -> (Vec<ClojureSourceFile>, Vec<String>) {
    let mut source_files = Vec::new();
    let mut warnings = Vec::new();

    let source_tree = WalkDir::new(&source_dir).min_depth(1);
    for entry in source_tree {
        match entry {
            Ok(entry) if entry.file_type().is_file() => {
                match entry.path().extension().and_then(OsStr::to_str) {
                    Some("clj" | "cljs" | "cljc") => {            //    v---- source_dir
                        let ns = entry.path()            // ~/dev/proj/src/com/my_org/core.clj
                            .strip_prefix(&source_dir)            //       com/my_org/core.clj
                            .expect("source root was not a prefix of file path")
                            .as_os_str()
                            .to_str()
                            .and_then(|path| {
                                let ns = path.rsplit_once('.')    //      (com/my_org/core|clj)
                                    .expect("file path with clojure extension did not contain '.'")
                                    .0                            //       com/my_org/core
                                    .replace(path::MAIN_SEPARATOR, ".") // com.my_org.core
                                    .replace('_', "-");           //       com.my-org.core
                                Some(ns)
                            });

                        let path = entry.path().as_os_str().to_str();

                        if let (Some(mut ns), Some(path)) = (ns, path) {
                            let path_start = ns.len();
                            ns.push_str(path);
                            source_files.push(ClojureSourceFile { entry: ns, path_start });
                        } else {
                            warnings.push(format!(
                                "path {} contains invalid utf8 characters",
                                &entry.path().display()
                            ));
                        }
                    }
                    _ => warnings.push(format!(
                        "{} is not a Clojure source file",
                        entry.path().display()
                    ))
                }
            }
            Err(error) => warnings.push(error.to_string()),
            _ => continue // skip non-files
        }
    }

    (source_files, warnings)
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
    rules: &[CompiledRule], source_files: &[ClojureSourceFile],
) -> Report {
    let mut report = Report::new(source_files);

    for file in source_files {
        for rule in rules {
            if rule.matches(file.namespace()) {
                report.rule_matched();
                match fs::read_to_string(file.path()) {
                    Ok(code) => rule.apply(file, code, &mut report),
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

    report
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
    fn new(file_list: &[ClojureSourceFile]) -> Self {
        Self {
            violations: vec![],
            warnings: vec![],
            namespaces_checked: file_list.len(),
            rules_matched: 0,
            files_skipped: 0,
        }
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
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.warnings.is_empty() {
            f.write_str("Warnings:\n\n")?;
            for warning in self.warnings.iter() {
                writeln!(f, "    {}, skipped file", warning)?;
            }
            f.write_str("\n\n")?;
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
                format!("Found {} rule violations", self.violations.len()).red()
            )?;
        }
        writeln!(
            f,
            "{:3} namespaces checked\n\
             {:3} rules matched\n\
             {:3} files skipped\n",
            self.namespaces_checked,
            self.rules_matched,
            self.warnings.len(),
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

    #[snippet(src)]
    snippet: SourceSpan,

    #[highlight(snippet, label("illegal reference occurs here"))]
    ref_location: SourceSpan,
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
    //cannot_access: Vec<NamespaceMatcher>,
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
