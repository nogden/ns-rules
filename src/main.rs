#![feature(iter_intersperse)]

use std::{env, ffi::OsStr, fs, iter, path::{self, Path}, str::FromStr};
use regex::Regex;
use walkdir::WalkDir;

fn main() {
    // process args
    let source_dir = env::args().skip(1).next().unwrap_or("src".to_string());

    // read config file
    // build rule set
    let rules = vec![Rule {
        namespace: "duka.boundary.*".parse().unwrap(),
        allow: vec!["duka.boundary.*".parse().unwrap()]
    }];

    // scan for clj cljc cljs files
    // determine namespace of each file
    let (source_files, warnings) = find_source_files(&source_dir);
    dbg!(&source_files, &warnings);


    // match namespace to rules
    // scan file for includes:
    //  (:require [duka.fulfillment.db])
    //  (require 'duka.fulfillment.db)
    //  (:use [duka.fulfillment.db])
    //  (use 'duka.fulfillment.db)
    //  duka.fulfillment.db/fetch-things
    // determine locations of rule violations
    let report = apply_rules(&rules, &source_files);

    // print rule violations
    dbg!(report);
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
            _ => continue
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

#[derive(Debug)]
struct Error;

fn apply_rules(
    rules: &[Rule], source_files: &[ClojureSourceFile],
) -> Report {
    let mut report = Report::new();

    for file in source_files {
        for rule in rules {
            if rule.matches(file.namespace()) {
                match fs::read_to_string(file.path()) {
                    Ok(code) => rule.apply(&code, &mut report),
                    Err(error) => report.warning(
                        format!("failed to read file {}: {}", file.path(), error)
                    )
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
}

impl Report {
    fn new() -> Self {
        Self { violations: vec![], warnings: vec![] }
    }

    fn warning(&mut self, warning: String) {
        self.warnings.push(warning);
    }
}

#[derive(Debug)]
struct Violation;

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
    fn matches(&self, namespace: &str) -> bool {
        self.namespace.matches(namespace)
    }

    fn apply(&self, code: &str, report: &mut Report) {
        dbg!("applying rule");
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
