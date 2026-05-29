use ruff_python_trivia::leading_indentation;
use ruff_source_file::UniversalNewlines;

use super::markdown;

/// Parsed reST field lists in a docstring.
pub(super) struct ParsedFieldLists<'a> {
    docstring: &'a str,
    lines: Vec<&'a str>,
    field_lists: Vec<FieldList>,
}

impl<'a> ParsedFieldLists<'a> {
    pub(super) fn parse(docstring: &'a str) -> Self {
        let lines = docstring
            .universal_newlines()
            .map(|line| line.as_str())
            .collect::<Vec<_>>();
        let field_lists = FieldList::parse_all(&lines);

        Self {
            docstring,
            lines,
            field_lists,
        }
    }

    pub(super) fn parameter_documentation(&self) -> Vec<ParameterDocumentation> {
        debug_assert!(self.docstring.is_empty() || !self.lines.is_empty());
        debug_assert!(self.field_lists.iter().all(|field_list| {
            field_list.start_line < field_list.end_line
                && field_list.end_line <= self.lines.len()
                && self
                    .lines
                    .get(field_list.start_line)
                    .is_some_and(|line| FieldStart::indentation(line) == field_list.indent)
        }));

        self.field_lists
            .iter()
            .flat_map(|field_list| &field_list.fields)
            .filter_map(|field| match field {
                Field::Parameter {
                    lookup_name,
                    description,
                    ..
                } if !description.is_empty() => Some(ParameterDocumentation {
                    name: lookup_name.clone(),
                    description: description.clone(),
                }),
                Field::Unknown {
                    name,
                    argument,
                    body,
                } => {
                    debug_assert!(!name.is_empty());
                    debug_assert!(!argument.contains('\n'));
                    debug_assert!(!body.starts_with('\n'));
                    None
                }
                _ => None,
            })
            .collect()
    }
}

/// Parameter documentation extracted from a reST field list.
pub(super) struct ParameterDocumentation {
    pub(super) name: String,
    pub(super) description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FieldList {
    start_line: usize,
    end_line: usize,
    indent: usize,
    fields: Vec<Field>,
}

impl FieldList {
    fn parse_all(lines: &[&str]) -> Vec<Self> {
        let mut field_lists = Vec::new();
        let mut code_examples = CodeExampleTracker::default();
        let mut index = 0;

        while let Some(line) = lines.get(index).copied() {
            if code_examples.contains_current_line(line) {
                index += 1;
            } else if FieldStart::parse(line).is_some() {
                let (field_list, next_index) = Self::parse(lines, index);
                if !field_list.fields.is_empty() {
                    field_lists.push(field_list);
                }
                index = next_index;
            } else {
                code_examples.observe_plaintext_line(line);
                index += 1;
            }
        }

        field_lists
    }

    fn parse(lines: &[&str], start: usize) -> (Self, usize) {
        debug_assert!(start < lines.len());
        debug_assert!(FieldStart::parse(lines[start]).is_some());

        let field_list_indent = FieldStart::indentation(lines[start]);
        let mut fields = Vec::new();
        let mut current: Option<FieldBuilder<'_>> = None;
        let mut index = start;

        while let Some(line) = lines.get(index).copied() {
            if let Some(start) = FieldStart::at_indent(line, field_list_indent) {
                if let Some(field) = current.take().map(FieldBuilder::finish) {
                    fields.push(field);
                }
                current = Some(FieldBuilder::new(start));
                index += 1;
                continue;
            }

            let Some(field) = &mut current else {
                break;
            };

            if line.trim().is_empty() {
                if Self::blank_line_belongs_to_field(lines, index, field.indent) {
                    field.lines.push(line);
                    index += 1;
                    continue;
                }
                break;
            }

            if FieldStart::indentation(line) > field.indent {
                field.lines.push(line);
                index += 1;
            } else {
                break;
            }
        }

        if let Some(field) = current.map(FieldBuilder::finish) {
            fields.push(field);
        }

        debug_assert!(index > start);

        (
            Self {
                start_line: start,
                end_line: index,
                indent: field_list_indent,
                fields,
            },
            index,
        )
    }

    fn blank_line_belongs_to_field(lines: &[&str], index: usize, indent: usize) -> bool {
        let mut next = index + 1;
        while let Some(line) = lines.get(next)
            && line.trim().is_empty()
        {
            next += 1;
        }

        lines.get(next).is_some_and(|line| {
            FieldStart::at_indent(line, indent).is_some() || FieldStart::indentation(line) > indent
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Field {
    Parameter {
        display_name: String,
        lookup_name: String,
        ty: Option<String>,
        description: String,
    },
    Unknown {
        name: String,
        argument: String,
        body: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FieldStart<'a> {
    indent: usize,
    kind: FieldKind<'a>,
    body: &'a str,
    raw: &'a str,
}

impl<'a> FieldStart<'a> {
    fn at_indent(line: &'a str, indent: usize) -> Option<Self> {
        (Self::indentation(line) == indent)
            .then(|| Self::parse(line))
            .flatten()
    }

    fn parse(line: &'a str) -> Option<Self> {
        let trimmed = line.trim_start();
        let after_opening_colon = trimmed.strip_prefix(':')?;
        let (name_and_argument, body) = after_opening_colon.split_once(':')?;
        if body
            .chars()
            .next()
            .is_some_and(|char| !char.is_whitespace())
        {
            return None;
        }

        let name_and_argument = name_and_argument.trim();
        if name_and_argument.is_empty() {
            return None;
        }

        let name_end = name_and_argument
            .find(char::is_whitespace)
            .unwrap_or(name_and_argument.len());
        let name = &name_and_argument[..name_end];
        let argument = name_and_argument[name_end..].trim();

        Some(Self {
            indent: Self::indentation(line),
            kind: FieldKind::parse(name, argument),
            body: body.trim_start(),
            raw: line,
        })
    }

    fn indentation(line: &str) -> usize {
        leading_indentation(line).len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldKind<'a> {
    Parameter {
        display_name: &'a str,
        lookup_name: &'a str,
        ty: Option<&'a str>,
    },
    Unknown {
        name: &'a str,
        argument: &'a str,
    },
}

impl<'a> FieldKind<'a> {
    fn parse(name: &'a str, argument: &'a str) -> Self {
        match name {
            "param" | "parameter" | "arg" | "argument" | "key" | "keyword" | "kwarg"
            | "kwparam" => Self::parse_parameter_argument(argument)
                .map(|(ty, name)| Self::Parameter {
                    display_name: name.display,
                    lookup_name: name.lookup,
                    ty,
                })
                .unwrap_or(Self::Unknown { name, argument }),
            _ => Self::Unknown { name, argument },
        }
    }

    fn parse_parameter_argument(argument: &'a str) -> Option<(Option<&'a str>, ParameterName<'a>)> {
        let argument = argument.trim();
        if argument.is_empty() {
            return None;
        }

        let name_start = argument
            .char_indices()
            .rev()
            .find_map(|(index, char)| char.is_whitespace().then_some(index + char.len_utf8()));

        if let Some(name_start) = name_start {
            let ty = argument[..name_start].trim();
            let name = Self::parse_parameter_name(&argument[name_start..])?;
            Some(((!ty.is_empty()).then_some(ty), name))
        } else {
            Some((None, Self::parse_parameter_name(argument)?))
        }
    }

    fn parse_parameter_name(name: &'a str) -> Option<ParameterName<'a>> {
        let display = name.trim();
        let lookup = display.trim_start_matches('*');
        (!lookup.is_empty()).then_some(ParameterName { display, lookup })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ParameterName<'a> {
    display: &'a str,
    lookup: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FieldBuilder<'a> {
    indent: usize,
    kind: FieldKind<'a>,
    body: &'a str,
    lines: Vec<&'a str>,
}

impl<'a> FieldBuilder<'a> {
    fn new(start: FieldStart<'a>) -> Self {
        Self {
            indent: start.indent,
            kind: start.kind,
            body: start.body,
            lines: vec![start.raw],
        }
    }

    fn finish(self) -> Field {
        let body = self.normalized_body();

        match self.kind {
            FieldKind::Parameter {
                display_name,
                lookup_name,
                ty,
            } => Field::Parameter {
                display_name: display_name.to_string(),
                lookup_name: lookup_name.to_string(),
                ty: ty.map(str::to_string),
                description: body,
            },
            FieldKind::Unknown { name, argument } => Field::Unknown {
                name: name.to_string(),
                argument: argument.to_string(),
                body,
            },
        }
    }

    fn normalized_body(&self) -> String {
        let continuation_indent = self
            .lines
            .iter()
            .skip(1)
            .filter(|line| !line.trim().is_empty())
            .map(|line| FieldStart::indentation(line))
            .min()
            .unwrap_or(0);

        let mut lines = Vec::with_capacity(self.lines.len());
        lines.push(self.body.trim_end().to_string());
        lines.extend(self.lines.iter().skip(1).map(|line| {
            if line.trim().is_empty() {
                String::new()
            } else {
                line.get(continuation_indent..)
                    .unwrap_or_default()
                    .trim_end()
                    .to_string()
            }
        }));

        let Some(start) = lines.iter().position(|line| !line.is_empty()) else {
            return String::new();
        };
        let end = lines
            .iter()
            .rposition(|line| !line.is_empty())
            .map_or(start, |index| index + 1);

        lines[start..end].join("\n")
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct CodeExampleTracker {
    markdown_fence: Option<String>,
    literal_block: LiteralBlockTracker,
    in_doctest: bool,
}

impl CodeExampleTracker {
    fn contains_current_line(&mut self, line: &str) -> bool {
        if let Some(fence) = &self.markdown_fence {
            if markdown::closes_fence(line, fence) {
                self.markdown_fence = None;
            }
            return true;
        }

        if self.literal_block.contains_current_line(line) {
            return true;
        }

        let trimmed = line.trim_start_matches(' ');
        if self.in_doctest {
            if trimmed.is_empty() {
                self.in_doctest = false;
            }
            return true;
        }

        if trimmed.starts_with(">>>") {
            self.in_doctest = true;
            return true;
        }

        if let Some(fence) = markdown::fence_start(line) {
            self.markdown_fence = Some(fence.to_string());
            return true;
        }

        false
    }

    fn observe_plaintext_line(&mut self, line: &str) {
        self.literal_block.observe_plaintext_line(line);
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct LiteralBlockTracker {
    pending: bool,
    active: bool,
    indent: usize,
}

impl LiteralBlockTracker {
    fn contains_current_line(&mut self, line: &str) -> bool {
        let trimmed = line.trim_start();
        let indent = FieldStart::indentation(line);

        if self.active && indent < self.indent && !trimmed.is_empty() {
            self.active = false;
            self.indent = 0;
        }

        if self.pending && !trimmed.is_empty() {
            self.pending = false;
            self.active = true;
            self.indent = indent;
        }

        self.active
    }

    fn observe_plaintext_line(&mut self, line: &str) {
        if !self.active && starts_literal_block(line.trim_start()) {
            self.pending = true;
        }
    }
}

fn starts_literal_block(line: &str) -> bool {
    let Some(prefix) = line.strip_suffix("::").or_else(|| {
        let (prefix, _language) = line.rsplit_once(' ')?;
        prefix.trim_end().strip_suffix("::")
    }) else {
        return false;
    };

    let directive = prefix
        .rsplit_once(' ')
        .and_then(|(prefix, directive)| prefix.strip_suffix("..").map(|_| directive));

    !matches!(
        directive,
        Some(
            "attention"
                | "caution"
                | "danger"
                | "error"
                | "hint"
                | "important"
                | "note"
                | "tip"
                | "warning"
                | "admonition"
                | "versionadded"
                | "version-added"
                | "versionchanged"
                | "version-changed"
                | "version-deprecated"
                | "deprecated"
                | "version-removed"
                | "versionremoved"
        )
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{Field, ParsedFieldLists};

    #[test]
    fn parameter_documentation_extracts_rest_parameters() {
        let docstring = r#"
        This is a function description.

        :param str param1: The first parameter description
        :param int param2: The second parameter description
            This is a continuation of param2 description.
        :param **kwargs: Extra keyword arguments.
        :returns: The return value description
        "#;
        let param_docs = parameter_documentation(docstring);

        assert_eq!(param_docs.len(), 3);
        assert_eq!(
            param_docs.get("param1").expect("param1 should exist"),
            "The first parameter description"
        );
        assert_eq!(
            param_docs.get("param2").expect("param2 should exist"),
            "The second parameter description\nThis is a continuation of param2 description."
        );
        assert_eq!(
            param_docs.get("kwargs").expect("kwargs should exist"),
            "Extra keyword arguments."
        );
    }

    #[test]
    fn parameter_documentation_supports_parameter_aliases() {
        let docstring = r#"
        :parameter first: The first parameter.
        :arg second: The second parameter.
        :argument third: The third parameter.
        :key fourth: The fourth parameter.
        :keyword fifth: The fifth parameter.
        :kwarg sixth: The sixth parameter.
        :kwparam seventh: The seventh parameter.
        "#;
        let param_docs = parameter_documentation(docstring);
        let expected = [
            ("first", "The first parameter."),
            ("second", "The second parameter."),
            ("third", "The third parameter."),
            ("fourth", "The fourth parameter."),
            ("fifth", "The fifth parameter."),
            ("sixth", "The sixth parameter."),
            ("seventh", "The seventh parameter."),
        ];

        assert_eq!(param_docs.len(), expected.len());
        for (name, description) in expected {
            assert_eq!(param_docs.get(name).map(String::as_str), Some(description));
        }
    }

    #[test]
    fn parameter_documentation_stops_at_field_boundaries() {
        let docstring = r#"
        :param param: The parameter description
        :type param: bool
        :returns value: The return value description
        :rtype: str
        "#;
        let param_docs = parameter_documentation(docstring);

        assert_eq!(param_docs.len(), 1);
        assert_eq!(
            param_docs.get("param").expect("param should exist"),
            "The parameter description"
        );
    }

    #[test]
    fn parameter_documentation_ignores_parameters_without_names_after_normalization() {
        assert!(parameter_documentation(":param **: Missing a parameter name.").is_empty());
    }

    #[test]
    fn parser_preserves_supported_and_unknown_fields() {
        let parsed = ParsedFieldLists::parse(
            "\
:param *args: Extra positional arguments.
:meta private:
:unknown with argument: Unknown description.",
        );

        assert_eq!(
            parsed.field_lists[0].fields,
            vec![
                Field::Parameter {
                    display_name: "*args".to_string(),
                    lookup_name: "args".to_string(),
                    ty: None,
                    description: "Extra positional arguments.".to_string()
                },
                Field::Unknown {
                    name: "meta".to_string(),
                    argument: "private".to_string(),
                    body: String::new()
                },
                Field::Unknown {
                    name: "unknown".to_string(),
                    argument: "with argument".to_string(),
                    body: "Unknown description.".to_string()
                },
            ]
        );
    }

    fn parameter_documentation(docstring: &str) -> HashMap<String, String> {
        ParsedFieldLists::parse(docstring)
            .parameter_documentation()
            .into_iter()
            .map(|parameter| (parameter.name, parameter.description))
            .collect()
    }
}
