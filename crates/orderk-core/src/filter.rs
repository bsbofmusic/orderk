use anyhow::{anyhow, Result};
use rusqlite::types::Value;

const MAX_FILTER_LEN: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterExpr {
    pub conditions: Vec<FilterCondition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterCondition {
    pub field: FilterField,
    pub op: FilterOp,
    pub value: FilterValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterField {
    Path,
    Title,
    Heading,
    Tag,
    HasCode,
    HasLink,
    HasTaskList,
    HasIncompleteTasks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterOp {
    Eq,
    Ne,
    Contains,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterValue {
    String(String),
    Bool(bool),
}

#[derive(Debug, Clone)]
pub struct FilterSql {
    pub sql: String,
    pub args: Vec<Value>,
}

pub fn parse_filter(raw: &str) -> Result<FilterExpr> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(anyhow!("invalid filter: empty expression"));
    }
    if raw.len() > MAX_FILTER_LEN {
        return Err(anyhow!("invalid filter: expression is too long"));
    }
    if raw.contains("||") || raw.contains('(') || raw.contains(')') {
        return Err(anyhow!("invalid filter: only flat && expressions are supported"));
    }

    let parts = split_conjunctions(raw)?;
    if parts.is_empty() {
        return Err(anyhow!("invalid filter: empty expression"));
    }
    let mut conditions = Vec::with_capacity(parts.len());
    for part in parts {
        conditions.push(parse_condition(part.trim())?);
    }
    Ok(FilterExpr { conditions })
}

pub fn compile_filter(raw: Option<&str>, alias: &str) -> Result<Option<FilterSql>> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let expr = parse_filter(raw)?;
    Ok(Some(expr.to_sql(alias)?))
}

impl FilterExpr {
    pub fn to_sql(&self, alias: &str) -> Result<FilterSql> {
        validate_alias(alias)?;
        let mut clauses = Vec::new();
        let mut args = Vec::new();
        for condition in &self.conditions {
            let compiled = condition_to_sql(condition, alias)?;
            clauses.push(compiled.sql);
            args.extend(compiled.args);
        }
        Ok(FilterSql {
            sql: clauses.join(" AND "),
            args,
        })
    }
}

fn condition_to_sql(condition: &FilterCondition, alias: &str) -> Result<FilterSql> {
    match condition.field {
        FilterField::Tag => compile_tag_condition(condition),
        FilterField::HasCode | FilterField::HasLink | FilterField::HasTaskList | FilterField::HasIncompleteTasks => {
            compile_bool_condition(condition, alias)
        }
        FilterField::Path | FilterField::Title | FilterField::Heading => compile_string_condition(condition, alias),
    }
}

fn compile_tag_condition(condition: &FilterCondition) -> Result<FilterSql> {
    if condition.op == FilterOp::Contains {
        return Err(anyhow!("invalid filter: tag does not support contains"));
    }
    let FilterValue::String(value) = &condition.value else {
        return Err(anyhow!("invalid filter: tag expects a string value"));
    };
    let exists = "EXISTS (SELECT 1 FROM json_each(c.tags_json) WHERE value = ?)";
    let sql = match condition.op {
        FilterOp::Eq => exists.to_string(),
        FilterOp::Ne => format!("NOT {exists}"),
        FilterOp::Contains => unreachable!(),
    };
    Ok(FilterSql { sql, args: vec![Value::Text(value.clone())] })
}

fn compile_bool_condition(condition: &FilterCondition, alias: &str) -> Result<FilterSql> {
    if condition.op == FilterOp::Contains {
        return Err(anyhow!("invalid filter: boolean fields do not support contains"));
    }
    let FilterValue::Bool(value) = &condition.value else {
        return Err(anyhow!("invalid filter: boolean field expects true or false"));
    };
    let value = *value;
    let column = bool_column(condition.field, alias)?;
    let op = match condition.op {
        FilterOp::Eq => "=",
        FilterOp::Ne => "!=",
        FilterOp::Contains => unreachable!(),
    };
    Ok(FilterSql {
        sql: format!("{column} {op} ?"),
        args: vec![Value::Integer(if value { 1 } else { 0 })],
    })
}

fn compile_string_condition(condition: &FilterCondition, alias: &str) -> Result<FilterSql> {
    let FilterValue::String(value) = &condition.value else {
        return Err(anyhow!("invalid filter: string field expects a quoted string value"));
    };
    let column = string_column(condition.field, alias)?;
    let sql = match condition.op {
        FilterOp::Eq => format!("coalesce({column}, '') = ?"),
        FilterOp::Ne => format!("coalesce({column}, '') != ?"),
        FilterOp::Contains => format!("instr(lower(coalesce({column}, '')), lower(?)) > 0"),
    };
    Ok(FilterSql { sql, args: vec![Value::Text(value.clone())] })
}

fn string_column(field: FilterField, alias: &str) -> Result<String> {
    let name = match field {
        FilterField::Path => "file_path",
        FilterField::Title => "title",
        FilterField::Heading => "heading",
        _ => return Err(anyhow!("invalid filter: field is not a string field")),
    };
    Ok(format!("{alias}.{name}"))
}

fn bool_column(field: FilterField, alias: &str) -> Result<String> {
    let name = match field {
        FilterField::HasCode => "has_code",
        FilterField::HasLink => "has_link",
        FilterField::HasTaskList => "has_task_list",
        FilterField::HasIncompleteTasks => "has_incomplete_tasks",
        _ => return Err(anyhow!("invalid filter: field is not a boolean field")),
    };
    Ok(format!("{alias}.{name}"))
}

fn validate_alias(alias: &str) -> Result<()> {
    if alias.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !alias.is_empty() {
        Ok(())
    } else {
        Err(anyhow!("invalid filter: internal SQL alias is invalid"))
    }
}

fn parse_condition(raw: &str) -> Result<FilterCondition> {
    if raw.is_empty() {
        return Err(anyhow!("invalid filter: empty condition"));
    }
    if let Some((left, right)) = split_once_operator(raw, " contains ")? {
        let field = parse_field(left.trim())?;
        let value = FilterValue::String(parse_quoted_string(right.trim())?);
        validate_operator_value(field, FilterOp::Contains, &value)?;
        return Ok(FilterCondition { field, op: FilterOp::Contains, value });
    }
    if let Some((left, right)) = split_once_operator(raw, "==")? {
        let field = parse_field(left.trim())?;
        let value = parse_value(right.trim())?;
        validate_operator_value(field, FilterOp::Eq, &value)?;
        return Ok(FilterCondition { field, op: FilterOp::Eq, value });
    }
    if let Some((left, right)) = split_once_operator(raw, "!=")? {
        let field = parse_field(left.trim())?;
        let value = parse_value(right.trim())?;
        validate_operator_value(field, FilterOp::Ne, &value)?;
        return Ok(FilterCondition { field, op: FilterOp::Ne, value });
    }
    Err(anyhow!("invalid filter: unsupported operator"))
}

fn validate_operator_value(field: FilterField, op: FilterOp, value: &FilterValue) -> Result<()> {
    match field {
        FilterField::Path | FilterField::Title | FilterField::Heading => {
            if matches!(value, FilterValue::String(_)) {
                Ok(())
            } else {
                Err(anyhow!("invalid filter: string field expects a quoted string value"))
            }
        }
        FilterField::Tag => {
            if op == FilterOp::Contains {
                return Err(anyhow!("invalid filter: tag does not support contains"));
            }
            if matches!(value, FilterValue::String(_)) {
                Ok(())
            } else {
                Err(anyhow!("invalid filter: tag expects a quoted string value"))
            }
        }
        FilterField::HasCode | FilterField::HasLink | FilterField::HasTaskList | FilterField::HasIncompleteTasks => {
            if op == FilterOp::Contains {
                return Err(anyhow!("invalid filter: boolean fields do not support contains"));
            }
            if matches!(value, FilterValue::Bool(_)) {
                Ok(())
            } else {
                Err(anyhow!("invalid filter: boolean field expects true or false"))
            }
        }
    }
}

fn parse_field(raw: &str) -> Result<FilterField> {
    match raw {
        "path" => Ok(FilterField::Path),
        "title" => Ok(FilterField::Title),
        "heading" => Ok(FilterField::Heading),
        "tag" => Ok(FilterField::Tag),
        "has_code" => Ok(FilterField::HasCode),
        "has_link" => Ok(FilterField::HasLink),
        "has_task_list" => Ok(FilterField::HasTaskList),
        "has_incomplete_tasks" => Ok(FilterField::HasIncompleteTasks),
        _ => Err(anyhow!("invalid filter: unknown field `{raw}`")),
    }
}

fn parse_value(raw: &str) -> Result<FilterValue> {
    match raw {
        "true" => Ok(FilterValue::Bool(true)),
        "false" => Ok(FilterValue::Bool(false)),
        _ => Ok(FilterValue::String(parse_quoted_string(raw)?)),
    }
}

fn parse_quoted_string(raw: &str) -> Result<String> {
    let mut chars = raw.chars();
    let quote = chars.next().ok_or_else(|| anyhow!("invalid filter: missing value"))?;
    if quote != '\'' && quote != '"' {
        return Err(anyhow!("invalid filter: string values must be quoted"));
    }
    let mut out = String::new();
    let mut escaped = false;
    let mut closed = false;
    for ch in chars {
        if escaped {
            out.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == quote {
            closed = true;
            break;
        } else {
            out.push(ch);
        }
    }
    if !closed || escaped {
        return Err(anyhow!("invalid filter: unterminated quoted string"));
    }
    let consumed_len = raw[..raw.len()].find_closing_quote_len(quote)?;
    if !raw[consumed_len..].trim().is_empty() {
        return Err(anyhow!("invalid filter: trailing characters after string value"));
    }
    Ok(out)
}

trait ClosingQuoteLen {
    fn find_closing_quote_len(&self, quote: char) -> Result<usize>;
}

impl ClosingQuoteLen for str {
    fn find_closing_quote_len(&self, quote: char) -> Result<usize> {
        let mut escaped = false;
        for (idx, ch) in self.char_indices().skip(1) {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == quote {
                return Ok(idx + ch.len_utf8());
            }
        }
        Err(anyhow!("invalid filter: unterminated quoted string"))
    }
}

fn split_conjunctions(raw: &str) -> Result<Vec<&str>> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let bytes = raw.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        let ch = raw[idx..].chars().next().unwrap();
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
        } else if raw[idx..].starts_with("&&") {
            parts.push(raw[start..idx].trim());
            idx += 2;
            start = idx;
            continue;
        }
        idx += ch.len_utf8();
    }
    if quote.is_some() || escaped {
        return Err(anyhow!("invalid filter: unterminated quoted string"));
    }
    parts.push(raw[start..].trim());
    if parts.iter().any(|part| part.is_empty()) {
        return Err(anyhow!("invalid filter: empty condition"));
    }
    Ok(parts)
}

fn split_once_operator<'a>(raw: &'a str, op: &str) -> Result<Option<(&'a str, &'a str)>> {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut found = None;
    let mut idx = 0usize;
    while idx < raw.len() {
        let ch = raw[idx..].chars().next().unwrap();
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
        } else if raw[idx..].starts_with(op) {
            if found.is_some() {
                return Err(anyhow!("invalid filter: duplicate operator"));
            }
            found = Some((idx, idx + op.len()));
            idx += op.len();
            continue;
        }
        idx += ch.len_utf8();
    }
    if quote.is_some() || escaped {
        return Err(anyhow!("invalid filter: unterminated quoted string"));
    }
    Ok(found.map(|(left_end, right_start)| (&raw[..left_end], &raw[right_start..])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flat_conjunction() {
        let expr = parse_filter("tag == 'rust' && has_code == true && path contains \"brain/\"").unwrap();
        assert_eq!(expr.conditions.len(), 3);
        assert_eq!(expr.conditions[0].field, FilterField::Tag);
        assert_eq!(expr.conditions[1].value, FilterValue::Bool(true));
        assert_eq!(expr.conditions[2].op, FilterOp::Contains);
    }

    #[test]
    fn rejects_unknown_or_unsupported_syntax() {
        assert!(parse_filter("unknown == 'x'").is_err());
        assert!(parse_filter("tag contains 'rust'").is_err());
        assert!(parse_filter("has_code contains 'true'").is_err());
        assert!(parse_filter("tag == rust").is_err());
        assert!(parse_filter("has_code == TRUE").is_err());
        assert!(parse_filter("tag == 'rust' || tag == 'go'").is_err());
        assert!(parse_filter("(tag == 'rust')").is_err());
    }

    #[test]
    fn parses_escaped_quotes_in_string_values() {
        let expr = parse_filter("path contains 'Bob\\'s notes' && title == \"A \\\"quoted\\\" title\"").unwrap();
        assert_eq!(expr.conditions[0].value, FilterValue::String("Bob's notes".to_string()));
        assert_eq!(expr.conditions[1].value, FilterValue::String("A \"quoted\" title".to_string()));
    }

    #[test]
    fn compiles_parameterized_sql() {
        let sql = compile_filter(Some("tag == 'rust' && has_code == true && path contains 'brain/'"), "c")
            .unwrap()
            .unwrap();
        assert!(sql.sql.contains("json_each(c.tags_json)"), "{}", sql.sql);
        assert!(sql.sql.contains("c.has_code = ?"), "{}", sql.sql);
        assert!(sql.sql.contains("instr(lower(coalesce(c.file_path"), "{}", sql.sql);
        assert_eq!(sql.args.len(), 3);
    }

    #[test]
    fn treats_injection_like_values_as_parameters() {
        let sql = compile_filter(Some("path contains \"'; DROP TABLE chunks; --\""), "c")
            .unwrap()
            .unwrap();
        assert!(!sql.sql.contains("DROP TABLE"));
        assert_eq!(sql.args, vec![Value::Text("'; DROP TABLE chunks; --".to_string())]);
    }
}
