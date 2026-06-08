//! Build a reqwest multipart Form from a Postman formdata body.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::model::Body;
use crate::subst::substitute;

/// A resolved description of the multipart form, ready to assemble.
#[derive(Debug, Clone)]
pub struct FormPlan {
    pub text_fields: Vec<(String, String)>,
    pub file_fields: Vec<(String, PathBuf)>,
}

/// Resolve a formdata Body into a FormPlan: substitute vars in text values,
/// validate file paths exist, skip disabled fields.
pub fn plan_form(body: &Body, vars: &HashMap<String, String>) -> Result<FormPlan, String> {
    let mut text_fields = Vec::new();
    let mut file_fields = Vec::new();
    for f in &body.formdata {
        if f.disabled {
            continue;
        }
        match f.kind.as_str() {
            "file" => {
                let src = f
                    .src
                    .as_ref()
                    .ok_or_else(|| format!("file field '{}' has no src", f.key))?;
                let path = PathBuf::from(substitute(src, vars));
                if !path.exists() {
                    return Err(format!(
                        "file not found for field '{}': {}",
                        f.key,
                        path.display()
                    ));
                }
                file_fields.push((substitute(&f.key, vars), path));
            }
            _ => {
                text_fields.push((substitute(&f.key, vars), substitute(&f.value, vars)));
            }
        }
    }
    Ok(FormPlan {
        text_fields,
        file_fields,
    })
}

/// Assemble a reqwest blocking multipart Form from a FormPlan.
pub fn build_reqwest_form(plan: &FormPlan) -> Result<reqwest::blocking::multipart::Form, String> {
    use reqwest::blocking::multipart::{Form, Part};
    let mut form = Form::new();
    for (k, v) in &plan.text_fields {
        form = form.text(k.clone(), v.clone());
    }
    for (k, path) in &plan.file_fields {
        let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let filename = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("upload")
            .to_string();
        let mime = mime_guess::from_path(path)
            .first_or_octet_stream()
            .to_string();
        let part = Part::bytes(bytes)
            .file_name(filename)
            .mime_str(&mime)
            .map_err(|e| e.to_string())?;
        form = form.part(k.clone(), part);
    }
    Ok(form)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Body, FormField};
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn formdata_body(file_path: &str) -> Body {
        Body {
            mode: "formdata".into(),
            raw: None,
            graphql: None,
            formdata: vec![
                FormField {
                    key: "title".into(),
                    kind: "text".into(),
                    value: "{{t}}".into(),
                    src: None,
                    disabled: false,
                },
                FormField {
                    key: "skip".into(),
                    kind: "text".into(),
                    value: "no".into(),
                    src: None,
                    disabled: true,
                },
                FormField {
                    key: "upload".into(),
                    kind: "file".into(),
                    value: String::new(),
                    src: Some(file_path.into()),
                    disabled: false,
                },
            ],
        }
    }

    #[test]
    fn collects_text_and_file_fields_skipping_disabled() {
        let dir = tempdir().unwrap();
        let fpath = dir.path().join("data.txt");
        std::fs::write(&fpath, b"hello").unwrap();
        let body = formdata_body(fpath.to_str().unwrap());
        let vars = HashMap::from([("t".to_string(), "Hi".to_string())]);
        let plan = plan_form(&body, &vars).unwrap();
        // disabled field excluded
        assert_eq!(
            plan.text_fields,
            vec![("title".to_string(), "Hi".to_string())]
        );
        assert_eq!(plan.file_fields.len(), 1);
        assert_eq!(plan.file_fields[0].0, "upload");
        assert_eq!(plan.file_fields[0].1, fpath);
    }

    #[test]
    fn errors_when_file_missing() {
        let body = formdata_body("/nonexistent/abc.bin");
        let err = plan_form(&body, &HashMap::new()).unwrap_err();
        assert!(
            err.contains("not found") || err.contains("No such"),
            "got: {err}"
        );
    }
}
