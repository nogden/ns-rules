use std::{fs, io, path::{Path, PathBuf}, collections::BTreeMap};
use thiserror::Error;
use miette::{Diagnostic};
use edn_rs::{Edn, EdnError};

use crate::{NamespaceMatcher, Report, Rule};

#[derive(Debug, Default)]
pub(crate) struct Config {
    pub source_dirs: Vec<String>,
    pub rules: Vec<Rule>,
}

#[derive(Debug, Error, Diagnostic)]
#[diagnostic(
    code(configuration_error),
    help("the configuration file is at {:?}", self.path)
)]
#[error("there was a problem loading the configuration file")]
pub(crate) struct Error {
    path: PathBuf,
    source: Problem,
}

#[derive(Debug, Error)]
pub(crate) enum Problem {
    #[error("the file could not be read")]
    ReadFailure {
        #[from]
        source: io::Error,
    },
    #[error("the file not does not contain valid EDN")]
    ParseFailure {
        #[from]
        source: EdnError,
    },
    #[error("the top level form must be an map")]
    NotAMap,
    #[error("the required key ':src-dirs' is missing")]
    MissingSrcDirs,
    #[error("':src-dirs' muat be a vector of strings")]
    BadSrcDirs,
    #[error("':src-dirs' must contain at least 1 directory")]
    EmptySrcDirs,
    #[error("the required key ':rules' is missing")]
    MissingRules,
    #[error("':rules' must be a vector containing an even number of forms")]
    BadRuleVector,
    #[error("the namespace pattern for rule {position} is invalid, namespace patterns must be symbols")]
    BadNsPattern {
        position: usize,
    },
    #[error("the rule '{ns_pattern}' is invalid, {detail}")]
    BadRule {
        ns_pattern: String,
        detail: String,
    }
}

pub(crate) fn read_file<P: AsRef<Path>>(
    path: P, report: &mut Report
) -> Result<Config, Error> {
    let config_edn: Edn = fs::read_to_string(&path)
        .map_err(|err| error(&path, err.into()))?.parse()
        .map_err(|err: EdnError| error(&path, err.into()))?;

    let mut config_map = if let Edn::Map(config_map) = config_edn {
        config_map.to_map()
    } else {
        Err(error(&path, Problem::NotAMap))?
    };

    let source_dirs = config_map.remove(":src-dirs")
        .ok_or(error(&path, Problem::MissingSrcDirs))?;

    let source_dirs = if let Edn::Vector(dir_list) = source_dirs {
        dir_list.to_vec()
            .into_iter()
            .map(expect_src_dir)
            .collect::<Result<Vec<String>, Problem>>()
            .map_err(|err| error(&path, err))?
    } else {
        Err(error(&path, Problem::BadSrcDirs))?
    };

    if source_dirs.is_empty() {
        Err(error(&path, Problem::EmptySrcDirs))?
    }

    let rules = config_map.remove(":rules")
        .ok_or(error(&path, Problem::MissingRules))?;

    let rules = if let Edn::Vector(rules) = rules {
        let rules = rules.to_vec();
        if rules.len() % 2 != 0 {
            Err(error(&path, Problem::BadRuleVector))?
        }

        let mut parsed_rules = vec![];
        for (i, rule_definition) in rules.chunks_exact(2).enumerate() {
            match rule_definition {
                [Edn::Symbol(ns_pattern), Edn::Map(rule)] => {
                    let rule = parse_rule(ns_pattern, rule.clone().to_map())
                        .map_err(|problem| error(&path, problem))?;

                    if let Some(rule) = rule {
                        parsed_rules.push(rule);
                    } else {
                        report.warn(format!("the rule for '{}' has no effect", ns_pattern));
                    }
                }
                [Edn::Symbol(ns_pattern), _] => {
                    Err(error(&path, Problem::BadRule {
                        ns_pattern: ns_pattern.clone(),
                        detail: "the rule body must be a map".into()
                    }))?
                }
                _ => {
                    Err(error(&path, Problem::BadNsPattern { position: i }))?
                }
            }
        }

        parsed_rules
    } else {
        Err(error(&path, Problem::BadRuleVector))?
    };

    Ok(Config { source_dirs, rules })
}

fn parse_rule(
    ns_pattern: &String, mut rule: BTreeMap<String, Edn>
) -> Result<Option<Rule>, Problem> {
    let ns_matcher: NamespaceMatcher = ns_pattern.parse()
        .map_err(|err: &str| Problem::BadRule {
            ns_pattern: ns_pattern.clone(),
            detail: err.into(),
        })?;

    let allow_list = if let Some(edn) = rule.remove(":restrict-to") {
        if let Edn::Vector(allow_list) = edn {
            let allow_list = allow_list.to_vec()
                .into_iter()
                .map(|allowed_ns| expect_ns_symbol(ns_pattern, allowed_ns))
                .collect::<Result<Vec<NamespaceMatcher>, Problem>>()?;

            if allow_list.is_empty() { None } else { Some(allow_list) }
        } else {
            Err(Problem::BadRule {
                ns_pattern: ns_pattern.into(),
                detail: "':restrict-to' must be a vector of symbols".into(),
            })?
        }
    } else {
        None
    };

    let rule = allow_list.map(|allow| Rule { namespace: ns_matcher, allow });

    Ok(rule)
}

fn expect_src_dir(edn: Edn) -> Result<String, Problem> {
    if let Edn::Str(s) = edn { Ok(s) } else { Err(Problem::BadSrcDirs) }
}

fn expect_ns_symbol(ns_pattern: &String, edn: Edn) -> Result<NamespaceMatcher, Problem> {
    if let Edn::Symbol(allowed_ns) = edn {
        allowed_ns.parse().map_err(|err: &str| Problem::BadRule {
            ns_pattern: ns_pattern.into(),
            detail: format!("the allowed namespace '{}' is invalid, {}", allowed_ns, err)
        })
    } else {
        Err(Problem::BadRule {
            ns_pattern: ns_pattern.into(),
            detail: "':restrict-to' must be a vector of symbols".into(),
        })
    }
}

fn error<P: AsRef<Path>>(path: P, problem: Problem) -> Error {
    Error { path: path.as_ref().into(), source: problem }
}
