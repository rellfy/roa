use crate::{Conflict, Error};
use http::uri::PathAndQuery;
use regex::{escape, Captures, Regex};
use std::collections::HashSet;
use std::str::FromStr;

const WILDCARD: &str = r"\*\{(?P<var>\w*)\}";
const VARIABLE: &str = r"/:(?P<var>\w*)/";

fn standardize_path(raw_path: &str) -> String {
    format!("/{}/", raw_path.trim_matches('/'))
}

fn must_build(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|err| {
        panic!(format!(
            r#"{}
                regex pattern {} is invalid, this is a bug of roa-router::path.
                please report it to https://github.com/Hexilee/roa"#,
            err, pattern
        ))
    })
}

#[derive(Clone)]
pub enum Path {
    Static(String),
    Dynamic(RegexPath),
}

#[derive(Clone)]
pub struct RegexPath {
    pub raw: String,
    pub vars: HashSet<String>,
    pub re: Regex,
}

impl Path {
    pub fn raw(&self) -> &str {
        match self {
            Path::Static(ref path) => path.as_str(),
            Path::Dynamic(ref re) => re.raw.as_str(),
        }
    }
}

impl FromStr for Path {
    type Err = Error;
    fn from_str(raw_path: &str) -> Result<Self, Self::Err> {
        let path = standardize_path(raw_path);
        Ok(match path_to_regexp(&path)? {
            None => Path::Static(path),
            Some((pattern, vars)) => Path::Dynamic(RegexPath {
                raw: path,
                vars,
                re: must_build(&pattern),
            }),
        })
    }
}

fn path_to_regexp(path: &str) -> Result<Option<(String, HashSet<String>)>, Error> {
    let mut pattern = escape(path.clone());
    let mut vars = HashSet::new();
    let wildcard_re = must_build(WILDCARD);
    let variable_re = must_build(VARIABLE);
    let wildcards: Vec<Captures> = wildcard_re.captures_iter(path).collect();
    let variables: Vec<Captures> = variable_re.captures_iter(path).collect();
    if wildcards.is_empty() && variables.is_empty() {
        return Ok(None);
    } else {
        let try_add_variable = |set: &mut HashSet<String>, variable: String| {
            if set.insert(variable.clone()) {
                Ok(())
            } else {
                Err(Conflict::Variable {
                    paths: (path.to_string(), path.to_string()),
                    var_name: variable,
                })
            }
        };
        for cap in wildcards {
            let variable = &cap["var"];
            if variable == r"" {
                return Err(Error::MissingVariable(path.to_string()));
            }
            let var = escape(variable);
            pattern = pattern.replace(
                &escape(&format!(r"*{{{}}}", variable)),
                &format!(r"(?P<{}>\w*)", &var),
            );
            try_add_variable(&mut vars, var)?;
        }
        for cap in variables {
            let variable = &cap["var"];
            if variable == "" {
                return Err(Error::MissingVariable(path.to_string()));
            }
            let var = escape(variable);
            pattern = pattern.replace(
                &escape(&format!(r":{}", variable)),
                &format!(r"(?P<{}>\w*)", &var),
            );
            try_add_variable(&mut vars, var)?;
        }
        Ok(Some((pattern, vars)))
    }
}

#[cfg(test)]
mod tests {
    use super::{must_build, path_to_regexp, VARIABLE, WILDCARD};
    use regex::Regex;
    use test_case::test_case;

    #[test_case("/:id/"; "pure dynamic")]
    #[test_case("/user/:id/"; "static prefix")]
    #[test_case("/user/:id/name"; "static prefix and suffix")]
    fn var_regex_match(path: &str) {
        let re = must_build(VARIABLE);
        let cap = re.captures(path);
        assert!(cap.is_some());
        assert_eq!("id", &cap.unwrap()["var"]);
    }

    #[test_case("/-:id/"; "invalid prefix")]
    #[test_case("/:i-d/"; "invalid variable name")]
    #[test_case("/:id-/"; "invalid suffix")]
    fn var_regex_mismatch(path: &str) {
        let re = must_build(VARIABLE);
        let cap = re.captures(path);
        assert!(cap.is_none());
    }

    #[test_case("*{id}"; "pure dynamic")]
    #[test_case("user-*{id}"; "static prefix")]
    #[test_case("user-*{id}-name"; "static prefix and suffix")]
    fn wildcard_regex_match(path: &str) {
        let re = must_build(WILDCARD);
        let cap = re.captures(path);
        assert!(cap.is_some());
        assert_eq!("id", &cap.unwrap()["var"]);
    }

    #[test_case("*"; "no variable")]
    #[test_case("*{-id}"; "invalid variable name")]
    fn wildcard_regex_mismatch(path: &str) {
        let re = must_build(WILDCARD);
        let cap = re.captures(path);
        assert!(cap.is_none());
    }

    #[test_case(r"/:id/" => r"/(?P<id>\w*)/"; "single variable")]
    fn path_to_regexp_dynamic_pattern(path: &str) -> String {
        path_to_regexp(path).unwrap().unwrap().0
    }
}
