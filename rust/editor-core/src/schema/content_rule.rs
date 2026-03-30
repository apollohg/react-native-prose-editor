/// A parsed content expression describing what children a node may contain.
///
/// Syntax follows ProseMirror conventions:
///   - `"block+"` — one or more nodes in the "block" group
///   - `"inline*"` — zero or more nodes in the "inline" group
///   - `"paragraph?"` — zero or one "paragraph" node
///   - `"paragraph block*"` — exactly one paragraph, then zero or more blocks
#[derive(Debug, Clone)]
pub struct ContentRule {
    pub parts: Vec<ContentPart>,
}

#[derive(Debug, Clone)]
pub struct ContentPart {
    /// The node type name or group name this part matches.
    pub group: String,
    /// Minimum number of matching children.
    pub min: u32,
    /// Maximum number of matching children (`None` = unbounded).
    pub max: Option<u32>,
}

impl ContentRule {
    /// Parse a ProseMirror-style content expression into a `ContentRule`.
    ///
    /// Each whitespace-separated token is parsed independently:
    ///   - `name+` → min=1, max=None
    ///   - `name*` → min=0, max=None
    ///   - `name?` → min=0, max=Some(1)
    ///   - `name`  → min=1, max=Some(1)
    pub fn parse(expr: &str) -> Result<Self, String> {
        let trimmed = expr.trim();
        if trimmed.is_empty() {
            return Ok(Self { parts: Vec::new() });
        }

        let parts = trimmed
            .split_whitespace()
            .map(|token| {
                if let Some(group) = token.strip_suffix('+') {
                    Ok(ContentPart {
                        group: group.to_string(),
                        min: 1,
                        max: None,
                    })
                } else if let Some(group) = token.strip_suffix('*') {
                    Ok(ContentPart {
                        group: group.to_string(),
                        min: 0,
                        max: None,
                    })
                } else if let Some(group) = token.strip_suffix('?') {
                    Ok(ContentPart {
                        group: group.to_string(),
                        min: 0,
                        max: Some(1),
                    })
                } else {
                    Ok(ContentPart {
                        group: token.to_string(),
                        min: 1,
                        max: Some(1),
                    })
                }
            })
            .collect::<Result<Vec<_>, String>>()?;

        Ok(Self { parts })
    }

    /// Returns `true` if this content rule allows no children at all.
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_plus_quantifier() {
        let rule = ContentRule::parse("block+").unwrap();
        assert_eq!(rule.parts.len(), 1);
        assert_eq!(rule.parts[0].group, "block");
        assert_eq!(rule.parts[0].min, 1);
        assert_eq!(rule.parts[0].max, None);
    }

    #[test]
    fn test_parse_star_quantifier() {
        let rule = ContentRule::parse("inline*").unwrap();
        assert_eq!(rule.parts.len(), 1);
        assert_eq!(rule.parts[0].group, "inline");
        assert_eq!(rule.parts[0].min, 0);
        assert_eq!(rule.parts[0].max, None);
    }

    #[test]
    fn test_parse_question_quantifier() {
        let rule = ContentRule::parse("heading?").unwrap();
        assert_eq!(rule.parts.len(), 1);
        assert_eq!(rule.parts[0].group, "heading");
        assert_eq!(rule.parts[0].min, 0);
        assert_eq!(rule.parts[0].max, Some(1));
    }

    #[test]
    fn test_parse_bare_name() {
        let rule = ContentRule::parse("paragraph").unwrap();
        assert_eq!(rule.parts.len(), 1);
        assert_eq!(rule.parts[0].group, "paragraph");
        assert_eq!(rule.parts[0].min, 1);
        assert_eq!(rule.parts[0].max, Some(1));
    }

    #[test]
    fn test_parse_compound_expression() {
        let rule = ContentRule::parse("paragraph block*").unwrap();
        assert_eq!(rule.parts.len(), 2);
        assert_eq!(rule.parts[0].group, "paragraph");
        assert_eq!(rule.parts[0].min, 1);
        assert_eq!(rule.parts[0].max, Some(1));
        assert_eq!(rule.parts[1].group, "block");
        assert_eq!(rule.parts[1].min, 0);
        assert_eq!(rule.parts[1].max, None);
    }

    #[test]
    fn test_parse_empty_string() {
        let rule = ContentRule::parse("").unwrap();
        assert!(rule.is_empty());
    }

    #[test]
    fn test_parse_whitespace_only() {
        let rule = ContentRule::parse("   ").unwrap();
        assert!(rule.is_empty());
    }
}
