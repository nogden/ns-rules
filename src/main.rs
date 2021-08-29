#![feature(iter_intersperse)]

use std::{str::FromStr, iter};
use regex::{Regex};

fn main() {
    // process args
    // read config file
    // build rule set
    // can for clj cljc cljs files
    // determine namespace of each file
    // match namespace to rules
    // scan file for includes:
    //  (:require [duka.fulfillment.db])
    //  (require 'duka.fulfillment.db)
    //  (:use [duka.fulfillment.db])
    //  (use 'duka.fulfillment.db)
    //  duka.fulfillment.db/fetch-things
    // determine locations of rule violations
    // print rule violations
}

#[derive(Debug)]
struct NamespaceMatcher(Regex);

#[derive(Debug)]
struct Rule {
    namespace: NamespaceMatcher,
    can_access: Vec<NamespaceMatcher>,
    cannot_access: Vec<NamespaceMatcher>,
}

impl NamespaceMatcher {
    fn matches(&self, namespace: &str) -> bool {
        self.0.is_match(namespace)
    }
}

impl FromStr for NamespaceMatcher {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() || s.contains(' ') {
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
    }
}
