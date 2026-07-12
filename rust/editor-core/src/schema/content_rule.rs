use std::cell::Cell;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

const MAX_NESTING_DEPTH: usize = 128;
const MAX_AUTOMATON_STATES: usize = 10_000;

pub(crate) struct WorkBudget {
    remaining: Cell<usize>,
}

#[derive(Debug)]
pub(crate) enum ContentRuleError {
    Semantic(String),
    ResourceExhausted(String),
}

impl ContentRuleError {
    fn semantic(message: impl Into<String>) -> Self {
        Self::Semantic(message.into())
    }

    fn resource(message: impl Into<String>) -> Self {
        Self::ResourceExhausted(message.into())
    }

    pub(crate) fn message(self) -> String {
        match self {
            Self::Semantic(message) | Self::ResourceExhausted(message) => message,
        }
    }
}

impl WorkBudget {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            remaining: Cell::new(limit),
        }
    }

    pub(crate) fn consume(&self) -> bool {
        let remaining = self.remaining.get();
        if remaining == 0 {
            return false;
        }
        self.remaining.set(remaining - 1);
        true
    }
}

/// A parsed ProseMirror-style content expression.
///
/// The expression is compiled to a small nondeterministic automaton so all
/// consumers use the same matching semantics (including ambiguous alternation
/// and repetition) instead of interpreting a linearized representation.
#[derive(Debug, Clone)]
pub struct ContentRule {
    states: Vec<State>,
    start: usize,
    accept: usize,
    symbols: BTreeSet<String>,
}

#[derive(Debug, Clone, Default)]
struct State {
    epsilon: Vec<usize>,
    transitions: Vec<(String, usize)>,
}

#[derive(Debug, Clone)]
enum Expr {
    Empty,
    Symbol(String),
    Sequence(Vec<Expr>),
    Alternation(Vec<Expr>),
    Repeat {
        expr: Box<Expr>,
        min: u32,
        max: Option<u32>,
    },
}

impl ContentRule {
    pub fn parse(source: &str) -> Result<Self, String> {
        Self::parse_with_budget(source, &WorkBudget::new(usize::MAX))
            .map_err(ContentRuleError::message)
    }

    pub(crate) fn parse_with_budget(
        source: &str,
        budget: &WorkBudget,
    ) -> Result<Self, ContentRuleError> {
        let mut parser = Parser::new(source, budget);
        let expression = if source.trim().is_empty() {
            Expr::Empty
        } else {
            let expression = parser.parse_alternation()?;
            parser.skip_whitespace();
            if !parser.at_end() {
                return Err(ContentRuleError::semantic(format!(
                    "unexpected '{}' at byte {}",
                    parser.peek().unwrap(),
                    parser.pos
                )));
            }
            expression
        };

        let symbols = parser.symbols;
        let mut compiler = Compiler::new(budget);
        let (start, accept) = compiler.compile(&expression)?;
        Ok(Self {
            states: compiler.states,
            start,
            accept,
            symbols,
        })
    }

    /// Whether this expression contains no symbols (the empty content rule).
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }

    /// Every node/group symbol referenced by this expression, in stable order.
    pub fn symbols(&self) -> impl Iterator<Item = &str> {
        self.symbols.iter().map(String::as_str)
    }

    /// Return whether the complete sequence is accepted by this expression.
    pub fn matches<T, F>(&self, children: &[T], mut symbol_matches: F) -> bool
    where
        F: FnMut(&T, &str) -> bool,
    {
        let states = self.states_after(children, &mut symbol_matches);
        states.contains(&self.accept)
    }

    pub(crate) fn matches_with_budget<T, F>(
        &self,
        children: &[T],
        mut symbol_matches: F,
        budget: &WorkBudget,
    ) -> Result<bool, ()>
    where
        F: FnMut(&T, &str) -> bool,
    {
        let mut current = self.epsilon_closure_budgeted([self.start], budget)?;
        for child in children {
            let mut next = HashSet::new();
            for state in current {
                if !budget.consume() {
                    return Err(());
                }
                for (symbol, target) in &self.states[state].transitions {
                    if !budget.consume() {
                        return Err(());
                    }
                    if symbol_matches(child, symbol) {
                        next.insert(*target);
                    }
                }
            }
            if next.is_empty() {
                return Ok(false);
            }
            current = self.epsilon_closure_budgeted(next, budget)?;
        }
        Ok(current.contains(&self.accept))
    }

    /// Return symbols accepted immediately after the supplied prefix.
    pub fn accepting_symbols_after<T, F>(&self, children: &[T], mut symbol_matches: F) -> Vec<&str>
    where
        F: FnMut(&T, &str) -> bool,
    {
        let states = self.states_after(children, &mut symbol_matches);
        let mut symbols = BTreeSet::new();
        for state in states {
            for (symbol, _) in &self.states[state].transitions {
                symbols.insert(symbol.as_str());
            }
        }
        symbols.into_iter().collect()
    }

    pub(crate) fn choose_matches<T, C, F>(
        &self,
        children: &[T],
        mut symbol_choice: F,
        budget: &WorkBudget,
    ) -> Result<Option<Vec<C>>, ()>
    where
        C: Copy,
        F: FnMut(&T, &str) -> Option<C>,
    {
        let mut current = self.epsilon_closure_budgeted([self.start], budget)?;
        let mut predecessors: Vec<HashMap<usize, (usize, C)>> = Vec::with_capacity(children.len());
        for child in children {
            let mut next = HashSet::new();
            let mut step = HashMap::new();
            for &state in &current {
                if !budget.consume() {
                    return Err(());
                }
                for (symbol, target) in &self.states[state].transitions {
                    if !budget.consume() {
                        return Err(());
                    }
                    let Some(choice) = symbol_choice(child, symbol) else {
                        continue;
                    };
                    for reached in self.epsilon_closure_budgeted([*target], budget)? {
                        if next.insert(reached) {
                            step.insert(reached, (state, choice));
                        }
                    }
                }
            }
            if next.is_empty() {
                return Ok(None);
            }
            predecessors.push(step);
            current = next;
        }
        if !current.contains(&self.accept) {
            return Ok(None);
        }
        let mut state = self.accept;
        let mut choices = Vec::with_capacity(children.len());
        for step in predecessors.iter().rev() {
            let Some((previous, choice)) = step.get(&state).copied() else {
                return Ok(None);
            };
            choices.push(choice);
            state = previous;
        }
        choices.reverse();
        Ok(Some(choices))
    }

    /// Return every symbol accepted at the start of the expression.
    pub fn initial_symbols(&self) -> Vec<&str> {
        let states = self.epsilon_closure([self.start]);
        let mut symbols = BTreeSet::new();
        for state in states {
            symbols.extend(
                self.states[state]
                    .transitions
                    .iter()
                    .map(|(symbol, _)| symbol.as_str()),
            );
        }
        symbols.into_iter().collect()
    }

    /// Find a shortest accepted sequence, choosing one concrete value for
    /// each traversed symbol. Returns `None` when no accepted path can be
    /// constructed from the choices supplied by the caller.
    pub fn minimal_match_with<T, F, C>(&self, mut choose: F, consume_work: C) -> Option<Vec<T>>
    where
        T: Clone,
        F: FnMut(&str) -> Option<T>,
        C: Fn() -> bool,
    {
        let mut pending = VecDeque::from([(self.start, Vec::new())]);
        let mut visited = HashSet::new();
        while let Some((state, values)) = pending.pop_front() {
            if !consume_work() {
                return None;
            }
            if !visited.insert(state) {
                continue;
            }
            if state == self.accept {
                return Some(values);
            }
            for target in self.states[state].epsilon.iter().rev() {
                pending.push_front((*target, values.clone()));
            }
            for (symbol, target) in &self.states[state].transitions {
                if !consume_work() {
                    return None;
                }
                if let Some(value) = choose(symbol) {
                    let mut next_values = values.clone();
                    next_values.push(value);
                    pending.push_back((*target, next_values));
                }
            }
        }
        None
    }

    /// Whether some accepted sequence can be built using only allowed symbols.
    pub fn is_constructible_with<F>(&self, mut symbol_is_constructible: F) -> bool
    where
        F: FnMut(&str) -> bool,
    {
        self.is_constructible_with_budget(
            &mut symbol_is_constructible,
            &WorkBudget::new(usize::MAX),
        )
        .unwrap_or(false)
    }

    pub(crate) fn is_constructible_with_budget<F>(
        &self,
        mut symbol_is_constructible: F,
        budget: &WorkBudget,
    ) -> Option<bool>
    where
        F: FnMut(&str) -> bool,
    {
        let mut pending = vec![self.start];
        let mut visited = HashSet::new();
        while let Some(state) = pending.pop() {
            if !budget.consume() {
                return None;
            }
            if !visited.insert(state) {
                continue;
            }
            if state == self.accept {
                return Some(true);
            }
            pending.extend(self.states[state].epsilon.iter().copied());
            for (symbol, target) in &self.states[state].transitions {
                if !budget.consume() {
                    return None;
                }
                if symbol_is_constructible(symbol) {
                    pending.push(*target);
                }
            }
        }
        Some(false)
    }

    fn states_after<T, F>(&self, children: &[T], symbol_matches: &mut F) -> HashSet<usize>
    where
        F: FnMut(&T, &str) -> bool,
    {
        let mut current = self.epsilon_closure([self.start]);
        for child in children {
            let mut next = HashSet::new();
            for state in current {
                for (symbol, target) in &self.states[state].transitions {
                    if symbol_matches(child, symbol) {
                        next.insert(*target);
                    }
                }
            }
            if next.is_empty() {
                return next;
            }
            current = self.epsilon_closure(next);
        }
        current
    }

    fn epsilon_closure<I>(&self, initial: I) -> HashSet<usize>
    where
        I: IntoIterator<Item = usize>,
    {
        let mut result = HashSet::new();
        let mut pending: Vec<usize> = initial.into_iter().collect();
        while let Some(state) = pending.pop() {
            if result.insert(state) {
                pending.extend(self.states[state].epsilon.iter().copied());
            }
        }
        result
    }

    fn epsilon_closure_budgeted<I>(
        &self,
        initial: I,
        budget: &WorkBudget,
    ) -> Result<HashSet<usize>, ()>
    where
        I: IntoIterator<Item = usize>,
    {
        let mut result = HashSet::new();
        let mut pending: Vec<usize> = initial.into_iter().collect();
        while let Some(state) = pending.pop() {
            if !budget.consume() {
                return Err(());
            }
            if result.insert(state) {
                pending.extend(self.states[state].epsilon.iter().copied());
            }
        }
        Ok(result)
    }
}

struct Parser<'a> {
    source: &'a str,
    pos: usize,
    symbols: BTreeSet<String>,
    depth: usize,
    budget: &'a WorkBudget,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str, budget: &'a WorkBudget) -> Self {
        Self {
            source,
            pos: 0,
            symbols: BTreeSet::new(),
            depth: 0,
            budget,
        }
    }

    fn consume_work(&self) -> Result<(), ContentRuleError> {
        if self.budget.consume() {
            Ok(())
        } else {
            Err(ContentRuleError::resource(
                "content expression work budget exceeded",
            ))
        }
    }

    fn parse_alternation(&mut self) -> Result<Expr, ContentRuleError> {
        self.consume_work()?;
        let mut alternatives = vec![self.parse_sequence()?];
        loop {
            self.skip_whitespace();
            if !self.consume('|') {
                break;
            }
            self.skip_whitespace();
            if self.at_end() || self.peek() == Some(')') || self.peek() == Some('|') {
                return Err(ContentRuleError::semantic(format!(
                    "missing expression after '|' at byte {}",
                    self.pos
                )));
            }
            alternatives.push(self.parse_sequence()?);
        }
        Ok(if alternatives.len() == 1 {
            alternatives.remove(0)
        } else {
            Expr::Alternation(alternatives)
        })
    }

    fn parse_sequence(&mut self) -> Result<Expr, ContentRuleError> {
        self.consume_work()?;
        let mut expressions = Vec::new();
        loop {
            self.skip_whitespace();
            if self.at_end() || matches!(self.peek(), Some(')') | Some('|')) {
                break;
            }
            expressions.push(self.parse_repeated()?);
        }
        if expressions.is_empty() {
            return Err(ContentRuleError::semantic(format!(
                "expected expression at byte {}",
                self.pos
            )));
        }
        Ok(if expressions.len() == 1 {
            expressions.remove(0)
        } else {
            Expr::Sequence(expressions)
        })
    }

    fn parse_repeated(&mut self) -> Result<Expr, ContentRuleError> {
        self.consume_work()?;
        let atom = self.parse_atom()?;
        self.skip_whitespace();
        let quantified = match self.peek() {
            Some('?') => {
                self.pos += 1;
                Expr::Repeat {
                    expr: Box::new(atom),
                    min: 0,
                    max: Some(1),
                }
            }
            Some('*') => {
                self.pos += 1;
                Expr::Repeat {
                    expr: Box::new(atom),
                    min: 0,
                    max: None,
                }
            }
            Some('+') => {
                self.pos += 1;
                Expr::Repeat {
                    expr: Box::new(atom),
                    min: 1,
                    max: None,
                }
            }
            Some('{') => self.parse_range(atom)?,
            _ => atom,
        };
        self.skip_whitespace();
        if matches!(self.peek(), Some('?') | Some('*') | Some('+') | Some('{')) {
            return Err(ContentRuleError::semantic(format!(
                "multiple quantifiers at byte {}",
                self.pos
            )));
        }
        Ok(quantified)
    }

    fn parse_atom(&mut self) -> Result<Expr, ContentRuleError> {
        self.consume_work()?;
        self.skip_whitespace();
        if self.consume('(') {
            if self.depth >= MAX_NESTING_DEPTH {
                return Err(ContentRuleError::resource(format!(
                    "content expression nesting exceeds {MAX_NESTING_DEPTH}"
                )));
            }
            self.depth += 1;
            self.skip_whitespace();
            if self.peek() == Some(')') {
                self.depth -= 1;
                return Err(ContentRuleError::semantic(format!(
                    "empty group at byte {}",
                    self.pos
                )));
            }
            let expression_result = self.parse_alternation();
            self.depth -= 1;
            let expression = expression_result?;
            self.skip_whitespace();
            if !self.consume(')') {
                return Err(ContentRuleError::semantic(format!(
                    "missing ')' at byte {}",
                    self.pos
                )));
            }
            return Ok(expression);
        }

        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            self.pos += self.peek().unwrap().len_utf8();
        }
        if start == self.pos {
            return Err(ContentRuleError::semantic(format!(
                "expected node or group name at byte {}",
                self.pos
            )));
        }
        let symbol = self.source[start..self.pos].to_string();
        self.symbols.insert(symbol.clone());
        Ok(Expr::Symbol(symbol))
    }

    fn parse_range(&mut self, atom: Expr) -> Result<Expr, ContentRuleError> {
        self.consume_work()?;
        self.consume('{');
        let min = self.parse_number()?;
        let max = if self.consume('}') {
            Some(min)
        } else {
            if !self.consume(',') {
                return Err(ContentRuleError::semantic(format!(
                    "expected ',' or '}}' at byte {}",
                    self.pos
                )));
            }
            if self.consume('}') {
                None
            } else {
                let max = self.parse_number()?;
                if !self.consume('}') {
                    return Err(ContentRuleError::semantic(format!(
                        "expected '}}' at byte {}",
                        self.pos
                    )));
                }
                Some(max)
            }
        };
        if max.is_some_and(|max| max < min) {
            return Err(ContentRuleError::semantic(format!(
                "range maximum is smaller than minimum at byte {}",
                self.pos
            )));
        }
        Ok(Expr::Repeat {
            expr: Box::new(atom),
            min,
            max,
        })
    }

    fn parse_number(&mut self) -> Result<u32, ContentRuleError> {
        self.consume_work()?;
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.pos += 1;
        }
        if start == self.pos {
            return Err(ContentRuleError::semantic(format!(
                "expected number at byte {}",
                self.pos
            )));
        }
        self.source[start..self.pos]
            .parse()
            .map_err(|_| ContentRuleError::semantic(format!("invalid count at byte {start}")))
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.pos += self.peek().unwrap().len_utf8();
        }
    }

    fn peek(&self) -> Option<char> {
        self.source[self.pos..].chars().next()
    }
    fn at_end(&self) -> bool {
        self.pos == self.source.len()
    }
    fn consume(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.pos += expected.len_utf8();
            true
        } else {
            false
        }
    }
}

struct Compiler<'a> {
    states: Vec<State>,
    budget: &'a WorkBudget,
}

impl<'a> Compiler<'a> {
    fn new(budget: &'a WorkBudget) -> Self {
        Self {
            states: Vec::new(),
            budget,
        }
    }

    fn state(&mut self) -> Result<usize, ContentRuleError> {
        if !self.budget.consume() {
            return Err(ContentRuleError::resource(
                "content expression work budget exceeded",
            ));
        }
        if self.states.len() >= MAX_AUTOMATON_STATES {
            return Err(ContentRuleError::resource(format!(
                "content expression exceeds {MAX_AUTOMATON_STATES} automaton states"
            )));
        }
        let index = self.states.len();
        self.states.push(State::default());
        Ok(index)
    }

    fn compile(&mut self, expression: &Expr) -> Result<(usize, usize), ContentRuleError> {
        self.compile_at_depth(expression, 0)
    }

    fn compile_at_depth(
        &mut self,
        expression: &Expr,
        depth: usize,
    ) -> Result<(usize, usize), ContentRuleError> {
        if depth > MAX_NESTING_DEPTH {
            return Err(ContentRuleError::resource(format!(
                "content expression nesting exceeds {MAX_NESTING_DEPTH}"
            )));
        }
        match expression {
            Expr::Empty => {
                let start = self.state()?;
                let end = self.state()?;
                self.states[start].epsilon.push(end);
                Ok((start, end))
            }
            Expr::Symbol(symbol) => {
                let start = self.state()?;
                let end = self.state()?;
                self.states[start].transitions.push((symbol.clone(), end));
                Ok((start, end))
            }
            Expr::Sequence(expressions) => {
                let start = self.state()?;
                let mut tail = start;
                for expression in expressions {
                    let (next_start, next_end) = self.compile_at_depth(expression, depth + 1)?;
                    self.states[tail].epsilon.push(next_start);
                    tail = next_end;
                }
                Ok((start, tail))
            }
            Expr::Alternation(expressions) => {
                let start = self.state()?;
                let end = self.state()?;
                for expression in expressions {
                    let (branch_start, branch_end) =
                        self.compile_at_depth(expression, depth + 1)?;
                    self.states[start].epsilon.push(branch_start);
                    self.states[branch_end].epsilon.push(end);
                }
                Ok((start, end))
            }
            Expr::Repeat { expr, min, max } => self.compile_repeat(expr, *min, *max, depth + 1),
        }
    }

    fn compile_repeat(
        &mut self,
        expression: &Expr,
        min: u32,
        max: Option<u32>,
        depth: usize,
    ) -> Result<(usize, usize), ContentRuleError> {
        // Bounds are expanded into automaton states. This generous cap prevents
        // hostile schemas from forcing impractical allocations.
        if max.unwrap_or(min) > 10_000 {
            return Err(ContentRuleError::resource(
                "content repetition bound exceeds 10000",
            ));
        }
        let start = self.state()?;
        let end = self.state()?;
        let mut tail = start;
        for _ in 0..min {
            let (item_start, item_end) = self.compile_at_depth(expression, depth)?;
            self.states[tail].epsilon.push(item_start);
            tail = item_end;
        }
        match max {
            Some(max) => {
                for _ in min..max {
                    self.states[tail].epsilon.push(end);
                    let (item_start, item_end) = self.compile_at_depth(expression, depth)?;
                    self.states[tail].epsilon.push(item_start);
                    tail = item_end;
                }
                self.states[tail].epsilon.push(end);
            }
            None => {
                self.states[tail].epsilon.push(end);
                let (item_start, item_end) = self.compile_at_depth(expression, depth)?;
                self.states[tail].epsilon.push(item_start);
                self.states[item_end].epsilon.push(tail);
            }
        }
        Ok((start, end))
    }
}

#[cfg(test)]
mod tests {
    use super::ContentRule;

    fn matches(rule: &ContentRule, children: &[&str]) -> bool {
        rule.matches(children, |child, symbol| *child == symbol)
    }

    #[test]
    fn matches_sequence_grouping_alternation_and_quantifiers() {
        let rule = ContentRule::parse("(paragraph | heading) block? text{2,3}").unwrap();
        assert!(matches(&rule, &["paragraph", "text", "text"]));
        assert!(matches(
            &rule,
            &["heading", "block", "text", "text", "text"]
        ));
        assert!(!matches(&rule, &["paragraph", "text"]));
        assert!(!matches(
            &rule,
            &["heading", "block", "text", "text", "text", "text"]
        ));
    }

    #[test]
    fn matches_unbounded_and_exact_ranges() {
        let rule = ContentRule::parse("a{2} b{1,}").unwrap();
        assert!(matches(&rule, &["a", "a", "b"]));
        assert!(matches(&rule, &["a", "a", "b", "b", "b"]));
        assert!(!matches(&rule, &["a", "b"]));
    }

    #[test]
    fn accepting_symbols_follow_all_ambiguous_paths() {
        let rule = ContentRule::parse("(a b | a c | d*) e").unwrap();
        assert_eq!(
            rule.accepting_symbols_after(&["a"], |child, symbol| *child == symbol),
            vec!["b", "c"]
        );
        assert_eq!(
            rule.accepting_symbols_after(&[] as &[&str], |child, symbol| *child == symbol),
            vec!["a", "d", "e"]
        );
    }

    #[test]
    fn rejects_excessive_parser_and_compiler_complexity() {
        let deeply_nested = format!("{}a{}", "(".repeat(129), ")".repeat(129));
        assert!(ContentRule::parse(&deeply_nested).is_err());
        assert!(ContentRule::parse("(a{101}){101}").is_err());
    }
}
