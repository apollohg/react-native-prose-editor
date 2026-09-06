#[derive(Clone, Copy)]
struct JsonSpan {
    start: usize,
    end: usize,
}

impl JsonSpan {
    fn len(self) -> Result<usize, SessionError> {
        self.end
            .checked_sub(self.start)
            .ok_or_else(|| create_scan_invalid("invalid create JSON span"))
    }
}

#[derive(Default)]
struct CreateRootSpans {
    schema: Option<JsonSpan>,
    initialization: Option<JsonSpan>,
}

#[derive(Default)]
struct InitializationSpans {
    kind: Option<ScannedInitializationKind>,
    json: Option<JsonSpan>,
    html: Option<JsonSpan>,
}

#[derive(Clone, Copy)]
enum ScannedInitializationKind {
    LocalJson,
    LocalHtml,
    Other,
}

struct CreateJsonScanner<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl<'a> CreateJsonScanner<'a> {
    fn new(json: &'a str) -> Self {
        Self {
            bytes: json.as_bytes(),
            index: 0,
        }
    }

    fn at(json: &'a str, index: usize) -> Self {
        Self {
            bytes: json.as_bytes(),
            index,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.index).copied()
    }

    fn bump(&mut self) -> Result<u8, SessionError> {
        let byte = self
            .peek()
            .ok_or_else(|| create_scan_invalid("unexpected end of create JSON"))?;
        self.index = self
            .index
            .checked_add(1)
            .ok_or_else(|| create_scan_invalid("create JSON index overflow"))?;
        Ok(byte)
    }

    fn skip_whitespace(&mut self) -> Result<(), SessionError> {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.bump()?;
        }
        Ok(())
    }

    fn consume(&mut self, expected: u8) -> Result<(), SessionError> {
        if self.bump()? != expected {
            return Err(create_scan_invalid("invalid create JSON syntax"));
        }
        Ok(())
    }

    fn scan_string(&mut self, expected: Option<&[u8]>) -> Result<(JsonSpan, bool), SessionError> {
        let start = self.index;
        self.consume(b'"')?;
        let mut expected_index = 0usize;
        let mut matches_expected = expected.is_some();
        loop {
            let byte = self.bump()?;
            match byte {
                b'"' => {
                    let matched = matches_expected
                        && expected.is_some_and(|value| expected_index == value.len());
                    return Ok((
                        JsonSpan {
                            start,
                            end: self.index,
                        },
                        matched,
                    ));
                }
                b'\\' => {
                    let escaped = self.bump()?;
                    let decoded = match escaped {
                        b'"' | b'\\' | b'/' => Some(escaped),
                        b'b' => Some(0x08),
                        b'f' => Some(0x0c),
                        b'n' => Some(b'\n'),
                        b'r' => Some(b'\r'),
                        b't' => Some(b'\t'),
                        b'u' => {
                            let mut code = 0u16;
                            for _ in 0..4 {
                                let digit = self.bump()?;
                                let value = match digit {
                                    b'0'..=b'9' => u16::from(digit - b'0'),
                                    b'a'..=b'f' => u16::from(digit - b'a') + 10,
                                    b'A'..=b'F' => u16::from(digit - b'A') + 10,
                                    _ => {
                                        return Err(create_scan_invalid(
                                            "invalid unicode escape in create JSON",
                                        ));
                                    }
                                };
                                code = code
                                    .checked_mul(16)
                                    .and_then(|current| current.checked_add(value))
                                    .ok_or_else(|| {
                                        create_scan_invalid("create JSON unicode escape overflow")
                                    })?;
                            }
                            u8::try_from(code).ok()
                        }
                        _ => return Err(create_scan_invalid("invalid escape in create JSON")),
                    };
                    if let Some(decoded) = decoded {
                        match_expected_byte(
                            expected,
                            &mut expected_index,
                            &mut matches_expected,
                            decoded,
                        )?;
                    } else {
                        matches_expected = false;
                    }
                }
                0x00..=0x1f => {
                    return Err(create_scan_invalid("unescaped control byte in create JSON"));
                }
                0x20..=0x7f => {
                    match_expected_byte(expected, &mut expected_index, &mut matches_expected, byte)?
                }
                _ => matches_expected = false,
            }
        }
    }

    fn scan_value(&mut self, depth: usize) -> Result<JsonSpan, SessionError> {
        enum Action {
            Value(usize),
            ObjectAfterValue(usize),
            ArrayAfterValue(usize),
        }

        let start = self.index;
        let mut actions = vec![Action::Value(depth)];
        while let Some(action) = actions.pop() {
            match action {
                Action::Value(depth) => {
                    if depth > CREATE_SCAN_MAX_DEPTH {
                        return Err(create_scan_invalid(
                            "create JSON nesting exceeds scanner limit",
                        ));
                    }
                    self.skip_whitespace()?;
                    match self.peek() {
                        Some(b'"') => {
                            self.scan_string(None)?;
                        }
                        Some(b'{') => {
                            self.bump()?;
                            self.skip_whitespace()?;
                            if self.peek() == Some(b'}') {
                                self.bump()?;
                                continue;
                            }
                            if self.peek() != Some(b'"') {
                                return Err(create_scan_invalid(
                                    "object key must be a string in create JSON",
                                ));
                            }
                            self.scan_string(None)?;
                            self.skip_whitespace()?;
                            self.consume(b':')?;
                            let child_depth = depth
                                .checked_add(1)
                                .ok_or_else(|| create_scan_invalid("create JSON depth overflow"))?;
                            actions.push(Action::ObjectAfterValue(depth));
                            actions.push(Action::Value(child_depth));
                        }
                        Some(b'[') => {
                            self.bump()?;
                            self.skip_whitespace()?;
                            if self.peek() == Some(b']') {
                                self.bump()?;
                                continue;
                            }
                            let child_depth = depth
                                .checked_add(1)
                                .ok_or_else(|| create_scan_invalid("create JSON depth overflow"))?;
                            actions.push(Action::ArrayAfterValue(depth));
                            actions.push(Action::Value(child_depth));
                        }
                        Some(b't') => self.scan_literal(b"true")?,
                        Some(b'f') => self.scan_literal(b"false")?,
                        Some(b'n') => self.scan_literal(b"null")?,
                        Some(b'-' | b'0'..=b'9') => self.scan_number()?,
                        _ => return Err(create_scan_invalid("invalid value in create JSON")),
                    }
                }
                Action::ObjectAfterValue(depth) => {
                    self.skip_whitespace()?;
                    match self.bump()? {
                        b'}' => {}
                        b',' => {
                            self.skip_whitespace()?;
                            if self.peek() != Some(b'"') {
                                return Err(create_scan_invalid(
                                    "object key must be a string in create JSON",
                                ));
                            }
                            self.scan_string(None)?;
                            self.skip_whitespace()?;
                            self.consume(b':')?;
                            let child_depth = depth
                                .checked_add(1)
                                .ok_or_else(|| create_scan_invalid("create JSON depth overflow"))?;
                            actions.push(Action::ObjectAfterValue(depth));
                            actions.push(Action::Value(child_depth));
                        }
                        _ => {
                            return Err(create_scan_invalid(
                                "invalid object delimiter in create JSON",
                            ))
                        }
                    }
                }
                Action::ArrayAfterValue(depth) => {
                    self.skip_whitespace()?;
                    match self.bump()? {
                        b']' => {}
                        b',' => {
                            let child_depth = depth
                                .checked_add(1)
                                .ok_or_else(|| create_scan_invalid("create JSON depth overflow"))?;
                            actions.push(Action::ArrayAfterValue(depth));
                            actions.push(Action::Value(child_depth));
                        }
                        _ => {
                            return Err(create_scan_invalid(
                                "invalid array delimiter in create JSON",
                            ))
                        }
                    }
                }
            }
        }
        Ok(JsonSpan {
            start,
            end: self.index,
        })
    }

    fn scan_literal(&mut self, literal: &[u8]) -> Result<(), SessionError> {
        let end = self
            .index
            .checked_add(literal.len())
            .ok_or_else(|| create_scan_invalid("create JSON literal index overflow"))?;
        if self.bytes.get(self.index..end) != Some(literal) {
            return Err(create_scan_invalid("invalid literal in create JSON"));
        }
        self.index = end;
        Ok(())
    }

    fn scan_number(&mut self) -> Result<(), SessionError> {
        if self.peek() == Some(b'-') {
            self.bump()?;
        }
        match self.peek() {
            Some(b'0') => {
                self.bump()?;
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    return Err(create_scan_invalid("leading zero in create JSON number"));
                }
            }
            Some(b'1'..=b'9') => {
                self.bump()?;
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.bump()?;
                }
            }
            _ => return Err(create_scan_invalid("invalid create JSON number")),
        }
        if self.peek() == Some(b'.') {
            self.bump()?;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(create_scan_invalid(
                    "invalid fraction in create JSON number",
                ));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.bump()?;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.bump()?;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.bump()?;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(create_scan_invalid(
                    "invalid exponent in create JSON number",
                ));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.bump()?;
            }
        }
        Ok(())
    }

    fn span_equals_ascii(&self, span: JsonSpan, expected: &[u8]) -> Result<bool, SessionError> {
        let mut scanner = Self {
            bytes: self.bytes,
            index: span.start,
        };
        let (rescanned, matched) = scanner.scan_string(Some(expected))?;
        if rescanned.end != span.end {
            return Err(create_scan_invalid("invalid create JSON string span"));
        }
        Ok(matched)
    }
}

fn match_expected_byte(
    expected: Option<&[u8]>,
    expected_index: &mut usize,
    matches_expected: &mut bool,
    decoded: u8,
) -> Result<(), SessionError> {
    if !*matches_expected {
        return Ok(());
    }
    if expected.and_then(|value| value.get(*expected_index)) != Some(&decoded) {
        *matches_expected = false;
        return Ok(());
    }
    *expected_index = expected_index
        .checked_add(1)
        .ok_or_else(|| create_scan_invalid("create JSON string index overflow"))?;
    Ok(())
}

fn create_scan_invalid(message: impl Into<String>) -> SessionError {
    config_invalid(None, message)
}

fn scan_create_root(json: &str) -> Result<CreateRootSpans, SessionError> {
    let mut scanner = CreateJsonScanner::new(json);
    scanner.skip_whitespace()?;
    scanner.consume(b'{')?;
    scanner.skip_whitespace()?;
    let mut spans = CreateRootSpans::default();
    if scanner.peek() == Some(b'}') {
        scanner.bump()?;
    } else {
        loop {
            if scanner.peek() != Some(b'"') {
                return Err(create_scan_invalid("create root key must be a string"));
            }
            let (key, _) = scanner.scan_string(None)?;
            scanner.skip_whitespace()?;
            scanner.consume(b':')?;
            let value = scanner.scan_value(1)?;
            if scanner.span_equals_ascii(key, b"schema")? {
                if spans.schema.replace(value).is_some() {
                    return Err(create_scan_invalid("duplicate schema field in create JSON"));
                }
            } else if scanner.span_equals_ascii(key, b"initialization")?
                && spans.initialization.replace(value).is_some()
            {
                return Err(create_scan_invalid(
                    "duplicate initialization field in create JSON",
                ));
            }
            scanner.skip_whitespace()?;
            match scanner.bump()? {
                b'}' => break,
                b',' => scanner.skip_whitespace()?,
                _ => return Err(create_scan_invalid("invalid create root delimiter")),
            }
        }
    }
    scanner.skip_whitespace()?;
    if scanner.index != scanner.bytes.len() {
        return Err(create_scan_invalid("trailing bytes in create JSON"));
    }
    Ok(spans)
}

fn scan_initialization(
    json: &str,
    initialization: JsonSpan,
) -> Result<InitializationSpans, SessionError> {
    let mut scanner = CreateJsonScanner::at(json, initialization.start);
    if scanner.peek() != Some(b'{') {
        return Ok(InitializationSpans::default());
    }
    scanner.consume(b'{')?;
    scanner.skip_whitespace()?;
    let mut type_span = None;
    let mut spans = InitializationSpans::default();
    if scanner.peek() == Some(b'}') {
        scanner.bump()?;
    } else {
        loop {
            if scanner.peek() != Some(b'"') {
                return Err(create_scan_invalid("initialization key must be a string"));
            }
            let (key, _) = scanner.scan_string(None)?;
            scanner.skip_whitespace()?;
            scanner.consume(b':')?;
            let value = scanner.scan_value(2)?;
            if scanner.span_equals_ascii(key, b"type")? {
                if type_span.replace(value).is_some() {
                    return Err(create_scan_invalid("duplicate initialization type"));
                }
            } else if scanner.span_equals_ascii(key, b"json")? {
                if spans.json.replace(value).is_some() {
                    return Err(create_scan_invalid("duplicate initialization json"));
                }
            } else if scanner.span_equals_ascii(key, b"html")?
                && spans.html.replace(value).is_some()
            {
                return Err(create_scan_invalid("duplicate initialization html"));
            }
            scanner.skip_whitespace()?;
            match scanner.bump()? {
                b'}' => break,
                b',' => scanner.skip_whitespace()?,
                _ => return Err(create_scan_invalid("invalid initialization delimiter")),
            }
        }
    }
    if scanner.index != initialization.end {
        return Err(create_scan_invalid("invalid initialization span"));
    }
    spans.kind = match type_span {
        Some(value) if scanner.bytes.get(value.start) == Some(&b'"') => {
            if scanner.span_equals_ascii(value, b"localJson")? {
                Some(ScannedInitializationKind::LocalJson)
            } else if scanner.span_equals_ascii(value, b"localHtml")? {
                Some(ScannedInitializationKind::LocalHtml)
            } else {
                Some(ScannedInitializationKind::Other)
            }
        }
        Some(_) => Some(ScannedInitializationKind::Other),
        None => None,
    };
    Ok(spans)
}

fn admit_create_retained_envelope(json: &str) -> Result<(), SessionError> {
    let root = scan_create_root(json)?;
    let initialization = root
        .initialization
        .map(|span| scan_initialization(json, span))
        .transpose()?
        .unwrap_or_default();
    let selected_payload = match initialization.kind {
        Some(ScannedInitializationKind::LocalJson) => initialization.json,
        Some(ScannedInitializationKind::LocalHtml) => initialization.html,
        Some(ScannedInitializationKind::Other) | None => None,
    };
    let mut retained = json.len();
    for deferred in [root.schema, selected_payload].into_iter().flatten() {
        retained = retained
            .checked_sub(deferred.len()?)
            .ok_or_else(|| create_scan_invalid("create retained-envelope underflow"))?;
    }
    admit_create_envelope_bytes(retained)
}
