use std::{fs, io, path::{Path, PathBuf}};
use thiserror::Error;
use miette::{Diagnostic};
use edn_rs::{Edn, EdnError};

use crate::Rule;

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
    #[error("the top level form must be an EDN map")]
    NotAMap,
    #[error("the required key ':src-dirs' is missing")]
    MissingSrcDirs,
    #[error("':src-dirs' muat be a vector of strings")]
    BadSrcDirs,
    #[error("':src-dirs' must contain at least 1 directory")]
    EmptySrcDirs,
}

pub(crate) fn read_file<P: AsRef<Path>>(path: P) -> Result<Config, Error> {
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

    // Parse rules

    Ok(Config { source_dirs, rules: vec![] })
}

fn expect_src_dir(edn: Edn) -> Result<String, Problem> {
    if let Edn::Str(s) = edn { Ok(s) } else { Err(Problem::BadSrcDirs) }
}

fn error<P: AsRef<Path>>(path: P, problem: Problem) -> Error {
    Error { path: path.as_ref().into(), source: problem }
}
