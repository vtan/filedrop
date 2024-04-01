use std::collections::HashMap;

use regex_lite::{Captures, Regex};

#[derive(Debug, Clone)]
pub struct Template {
    template: String,
    vars: Vec<TemplateVar>,
}

#[derive(Debug, Clone)]
struct TemplateVar {
    offset: usize,
    var: String,
    unescaped: bool,
}

impl Template {
    pub fn one(str: &str) -> Self {
        let var_regex = Regex::new(r"(\@)?\{([a-zA-Z0-9_]+)\}").unwrap();

        let mut vars = vec![];
        let mut found = true;
        let mut template = str.to_string();

        while found {
            found = false;
            template = var_regex
                .replace(&template, |captures: &Captures| {
                    let offset = captures.get(0).unwrap().start();
                    let unescaped = captures.get(1).is_some();
                    let var = captures.get(2).unwrap().as_str().to_string();

                    let template_var = TemplateVar {
                        offset,
                        var,
                        unescaped,
                    };
                    vars.push(template_var);
                    found = true;

                    ""
                })
                .to_string();
        }

        Self { template, vars }
    }

    pub fn many(str: &str) -> Vec<Self> {
        let separator = Regex::new(r"(?m)^---$").unwrap();
        separator.split(str).map(Self::one).collect()
    }

    pub fn render(&self, replacements: &HashMap<String, impl AsRef<str>>) -> String {
        let mut str = self.template.clone();
        for var in self.vars.iter().rev() {
            if let Some(replacement) = replacements.get(&var.var) {
                if var.unescaped {
                    str.insert_str(var.offset, replacement.as_ref());
                } else {
                    let replacement = html_escape(replacement.as_ref());
                    str.insert_str(var.offset, &replacement);
                }
            } else {
                panic!("Undefined variable {}", var.var);
            }
        }
        str
    }

    pub fn render_many(
        &self,
        replacements: impl Iterator<Item = HashMap<String, impl AsRef<str>>>,
    ) -> String {
        replacements.map(|r| self.render(&r)).collect()
    }
}

fn html_escape(str: &str) -> String {
    str.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
