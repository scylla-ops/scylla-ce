use anyhow::{Result, anyhow};
use serde_json::Value;
use std::collections::BTreeMap;

pub type Vars = BTreeMap<String, String>;

pub fn render_str(text: &str, vars: &Vars) -> Result<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        let end = tail
            .find('}')
            .ok_or_else(|| anyhow!("unclosed placeholder in {text:?}"))?;
        let name = &tail[..end];
        let value = vars
            .get(name)
            .ok_or_else(|| anyhow!("unknown variable {name:?}"))?;
        out.push_str(value);
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

pub fn render(value: &Value, vars: &Vars) -> Result<Value> {
    Ok(match value {
        Value::String(s) => Value::String(render_str(s, vars)?),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| render(item, vars))
                .collect::<Result<_>>()?,
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| Ok((k.clone(), render(v, vars)?)))
                .collect::<Result<_>>()?,
        ),
        other => other.clone(),
    })
}

pub fn pick(value: &Value, pointer: &str) -> Result<String> {
    let found = value
        .pointer(pointer)
        .ok_or_else(|| anyhow!("no value at {pointer} in {value}"))?;
    Ok(match found {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_placeholder_takes_the_value_of_its_variable() {
        let vars = Vars::from([("org".to_string(), "01ABC".to_string())]);
        let out = render(
            &json!({"id": {"value": "${org}"}, "n": ["x-${org}"]}),
            &vars,
        )
        .unwrap();
        assert_eq!(out, json!({"id": {"value": "01ABC"}, "n": ["x-01ABC"]}));
    }

    #[test]
    fn an_unknown_variable_is_an_error() {
        assert!(render_str("${nope}", &Vars::new()).is_err());
    }
}
