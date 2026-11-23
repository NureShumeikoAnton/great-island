//! The calculator's arithmetic. What is typed is a list of tokens (numbers, operators,
//! brackets); it is counted the standard way: the shunting-yard algorithm turns it into reverse
//! Polish notation, and a stack counts that. × and ÷ bind before + and −, brackets first.
//!
//! On top of the expression, the calculator's habits: `=` again repeats the last operation; an
//! operator right after another replaces it; a digit after a result starts over; a digit right
//! after a closing bracket multiplies.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
}

impl Op {
    fn sign(self) -> char {
        match self {
            Self::Add => '+',
            Self::Sub => '−',
            Self::Mul => '×',
            Self::Div => '÷',
        }
    }

    fn precedence(self) -> u8 {
        match self {
            Self::Add | Self::Sub => 1,
            Self::Mul | Self::Div => 2,
        }
    }

    fn apply(self, a: f64, b: f64) -> Result<f64, Problem> {
        match self {
            Self::Add => Ok(a + b),
            Self::Sub => Ok(a - b),
            Self::Mul => Ok(a * b),
            Self::Div if b == 0.0 && a == 0.0 => Err(Problem::Undefined),
            Self::Div if b == 0.0 => Err(Problem::DivideByZero),
            Self::Div => Ok(a / b),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    DivideByZero,
    /// Zero divided by zero.
    Undefined,
    Overflow,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Separators {
    pub decimal: String,
    /// Between groups of three digits.
    pub group: String,
}

/// A typed number keeps more digits than this out.
const MAX_DIGITS: usize = 16;

#[derive(Clone, Debug, PartialEq)]
enum Token {
    /// As typed: `.` and a leading `-`, trailing zeros kept.
    Typed(String),
    /// Counted by the calculator (a result, a percentage): shown in the number format.
    Value(f64),
    Op(Op),
    Open,
    Close,
}

impl Token {
    fn is_number(&self) -> bool {
        matches!(self, Self::Typed(_) | Self::Value(_))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Engine {
    tokens: Vec<Token>,
    /// After `=`: the expression the result came from, for the line over it. `tokens` is the result.
    solved: Option<Vec<Token>>,
    /// `=` again applies this to the result.
    repeat: Option<(Op, f64)>,
    problem: Option<Problem>,
}

impl Engine {
    pub fn digit(&mut self, d: u8) {
        self.fresh();
        let digit = char::from(b'0' + d.min(9));
        match self.tokens.last_mut() {
            Some(Token::Typed(entry)) => {
                if entry.bytes().filter(u8::is_ascii_digit).count() >= MAX_DIGITS {
                    return;
                }
                match entry.as_str() {
                    "0" => *entry = digit.into(),
                    "-0" => *entry = format!("-{digit}"),
                    _ => entry.push(digit),
                }
            }
            Some(Token::Value(_)) => {
                self.tokens.pop();
                self.tokens.push(Token::Typed(digit.into()));
            }
            _ => self.start_number(digit.into()),
        }
    }

    pub fn point(&mut self) {
        self.fresh();
        match self.tokens.last_mut() {
            Some(Token::Typed(entry)) if !entry.contains('.') => entry.push('.'),
            Some(Token::Typed(_)) => {}
            Some(Token::Value(_)) => {
                self.tokens.pop();
                self.tokens.push(Token::Typed("0.".into()));
            }
            _ => self.start_number("0.".into()),
        }
    }

    pub fn op(&mut self, op: Op) {
        if self.problem.is_some() {
            return;
        }
        self.solved = None;
        match self.tokens.last_mut() {
            Some(Token::Op(last)) => *last = op,
            // An operator can't follow an opening bracket.
            Some(Token::Open) => {}
            None => self
                .tokens
                .extend([Token::Typed("0".into()), Token::Op(op)]),
            Some(_) => self.tokens.push(Token::Op(op)),
        }
    }

    /// From the keyboard only: the page has no bracket keys.
    pub fn open(&mut self) {
        self.fresh();
        if self
            .tokens
            .last()
            .is_some_and(|t| t.is_number() || *t == Token::Close)
        {
            self.tokens.push(Token::Op(Op::Mul));
        }
        self.tokens.push(Token::Open);
    }

    pub fn close(&mut self) {
        if self.problem.is_some() || self.solved.is_some() || depth(&self.tokens) == 0 {
            return;
        }
        if self
            .tokens
            .last()
            .is_some_and(|t| t.is_number() || *t == Token::Close)
        {
            self.tokens.push(Token::Close);
        }
    }

    pub fn equals(&mut self) {
        if self.problem.is_some() {
            return;
        }
        if self.tokens.iter().all(Token::is_number) {
            let Some((op, operand)) = self.repeat else {
                return;
            };
            let base = self.shown_value();
            self.solve(vec![
                Token::Value(base),
                Token::Op(op),
                Token::Value(operand),
            ]);
            return;
        }
        let mut tokens = std::mem::take(&mut self.tokens);
        while tokens.last() == Some(&Token::Open) {
            tokens.pop();
        }
        if tokens.is_empty() {
            return;
        }
        if matches!(tokens.last(), Some(Token::Op(_))) {
            tokens.push(Token::Value(preview(&tokens)));
        }
        tokens.extend(std::iter::repeat_n(Token::Close, depth(&tokens)));
        if let Some(at) = last_outer_op(&tokens)
            && let Token::Op(op) = tokens[at]
        {
            self.repeat = count(&tokens[at + 1..]).ok().map(|operand| (op, operand));
        }
        self.solve(tokens);
    }

    /// The number at the end as a share: of what stands before it for + and − ("200 + 10 %" is
    /// 220), of 100 otherwise.
    pub fn percent(&mut self) {
        if self.problem.is_some() {
            return;
        }
        let Some(last) = self.tokens.last() else {
            return;
        };
        if !last.is_number() {
            return;
        }
        let x = number(last);
        let at = self.tokens.len() - 1;
        let share = match at.checked_sub(1).map(|i| &self.tokens[i]) {
            Some(&Token::Op(op @ (Op::Add | Op::Sub))) => {
                let start = chunk_start(&self.tokens, at - 1, op);
                count(&self.tokens[start..at - 1]).unwrap_or(0.0) * x / 100.0
            }
            _ => x / 100.0,
        };
        self.solved = None;
        self.tokens[at] = Token::Value(tidy(share));
    }

    pub fn negate(&mut self) {
        if self.problem.is_some() {
            return;
        }
        self.solved = None;
        match self.tokens.last_mut() {
            Some(Token::Typed(entry)) => match entry.strip_prefix('-') {
                Some(rest) => *entry = rest.into(),
                None => entry.insert(0, '-'),
            },
            Some(Token::Value(value)) => *value = -*value,
            Some(Token::Close) => {}
            _ => self.tokens.push(Token::Typed("-0".into())),
        }
    }

    pub fn backspace(&mut self) {
        if self.problem.is_some() {
            return self.clear();
        }
        if self.solved.is_some() {
            return;
        }
        match self.tokens.last_mut() {
            Some(Token::Typed(entry)) => {
                entry.pop();
                if entry.is_empty() || entry == "-" {
                    self.tokens.pop();
                }
            }
            Some(_) => {
                self.tokens.pop();
            }
            None => {}
        }
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn problem(&self) -> Option<Problem> {
        self.problem
    }

    /// The big number: what is typed, the result, or the value so far right after an operator.
    pub fn display(&self, sep: &Separators) -> String {
        match self.tokens.last() {
            Some(Token::Typed(entry)) => group(entry, sep),
            _ => format(self.shown_value(), sep),
        }
    }

    /// The line over it: `12 + 3 ×` while typing, `12 + 3 × 2 =` over a result.
    pub fn expression(&self, sep: &Separators) -> String {
        if let Some(solved) = &self.solved {
            return format!("{} =", line(solved, sep));
        }
        let shown = match self.tokens.last() {
            Some(t) if t.is_number() => &self.tokens[..self.tokens.len() - 1],
            _ => &self.tokens[..],
        };
        line(shown, sep)
    }

    /// The shown number as other programs read it: no group separators, an ASCII minus.
    pub fn copied(&self, sep: &Separators) -> Option<String> {
        if self.problem.is_some() {
            return None;
        }
        let shown = self.display(sep);
        Some(shown.replace(sep.group.as_str(), "").replace('−', "-"))
    }

    fn shown_value(&self) -> f64 {
        match self.tokens.last() {
            None | Some(Token::Open) => 0.0,
            Some(t) if t.is_number() => number(t),
            Some(Token::Close) => {
                let open = matching_open(&self.tokens);
                count(&self.tokens[open..]).unwrap_or(0.0)
            }
            Some(_) => preview(&self.tokens),
        }
    }

    fn start_number(&mut self, entry: String) {
        if self.tokens.last() == Some(&Token::Close) {
            self.tokens.push(Token::Op(Op::Mul));
        }
        self.tokens.push(Token::Typed(entry));
    }

    fn solve(&mut self, tokens: Vec<Token>) {
        match count(&tokens) {
            Ok(value) => {
                self.tokens = vec![Token::Value(value)];
                self.solved = Some(tokens);
            }
            Err(problem) => {
                self.problem = Some(problem);
                self.tokens.clear();
                self.solved = None;
            }
        }
    }

    /// Before a digit, a point or a bracket: a problem or a result is gone, but `=` still repeats
    /// the last operation on the new number.
    fn fresh(&mut self) {
        if self.problem.is_some() || self.solved.is_some() {
            let repeat = self.repeat.filter(|_| self.problem.is_none());
            self.clear();
            self.repeat = repeat;
        }
    }
}

fn number(token: &Token) -> f64 {
    match token {
        Token::Typed(entry) => entry.parse().unwrap_or(0.0),
        Token::Value(value) => *value,
        _ => 0.0,
    }
}

/// Brackets opened and not closed yet.
fn depth(tokens: &[Token]) -> usize {
    tokens.iter().fold(0usize, |depth, t| match t {
        Token::Open => depth + 1,
        Token::Close => depth.saturating_sub(1),
        _ => depth,
    })
}

/// Where the operand of the operator at `at` begins: past the last operator that binds looser
/// at the same depth, or past the bracket it is in.
fn chunk_start(tokens: &[Token], at: usize, op: Op) -> usize {
    let mut depth = 0usize;
    for i in (0..at).rev() {
        match &tokens[i] {
            Token::Close => depth += 1,
            Token::Open if depth == 0 => return i + 1,
            Token::Open => depth -= 1,
            Token::Op(other) if depth == 0 && other.precedence() < op.precedence() => {
                return i + 1;
            }
            _ => {}
        }
    }
    0
}

fn matching_open(tokens: &[Token]) -> usize {
    let mut depth = 0usize;
    for i in (0..tokens.len()).rev() {
        match tokens[i] {
            Token::Close => depth += 1,
            Token::Open => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    0
}

fn last_outer_op(tokens: &[Token]) -> Option<usize> {
    let mut depth = 0usize;
    for i in (0..tokens.len()).rev() {
        match tokens[i] {
            Token::Close => depth += 1,
            Token::Open => depth = depth.saturating_sub(1),
            Token::Op(_) if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// Right after an operator: the value of what it applies to, as far as precedence lets it be
/// known ("2 + 3 ×" shows 3, "2 + 3 +" shows 5).
fn preview(tokens: &[Token]) -> f64 {
    let at = tokens.len() - 1;
    let Token::Op(op) = tokens[at] else {
        return 0.0;
    };
    count(&tokens[chunk_start(tokens, at, op)..at]).unwrap_or(0.0)
}

/// Shunting-yard: infix tokens to reverse Polish notation. Operators of equal precedence go left
/// to right.
fn to_rpn(tokens: &[Token]) -> Vec<&Token> {
    let mut out = Vec::with_capacity(tokens.len());
    let mut stack: Vec<&Token> = Vec::new();
    for token in tokens {
        match token {
            Token::Typed(_) | Token::Value(_) => out.push(token),
            Token::Op(op) => {
                while let Some(Token::Op(top)) = stack.last()
                    && top.precedence() >= op.precedence()
                {
                    out.extend(stack.pop());
                }
                stack.push(token);
            }
            Token::Open => stack.push(token),
            Token::Close => {
                while let Some(top) = stack.pop() {
                    if *top == Token::Open {
                        break;
                    }
                    out.push(top);
                }
            }
        }
    }
    out.extend(stack.into_iter().rev().filter(|t| **t != Token::Open));
    out
}

/// The expression's value: its reverse Polish notation counted on a stack. A malformed
/// expression (which the keys can't type) counts as what its stack holds.
fn count(tokens: &[Token]) -> Result<f64, Problem> {
    let mut stack: Vec<f64> = Vec::new();
    for token in to_rpn(tokens) {
        match token {
            Token::Op(op) => {
                let b = stack.pop().unwrap_or(0.0);
                let a = stack.pop().unwrap_or(0.0);
                let value = op.apply(a, b)?;
                if !value.is_finite() {
                    return Err(Problem::Overflow);
                }
                stack.push(value);
            }
            operand => stack.push(number(operand)),
        }
    }
    Ok(tidy(stack.pop().unwrap_or(0.0)))
}

fn line(tokens: &[Token], sep: &Separators) -> String {
    let mut out = String::new();
    for (i, token) in tokens.iter().enumerate() {
        let glued = i == 0 || tokens[i - 1] == Token::Open || *token == Token::Close;
        if !glued {
            out.push(' ');
        }
        match token {
            Token::Typed(entry) => out.push_str(&group(entry, sep)),
            Token::Value(value) => out.push_str(&format(*value, sep)),
            Token::Op(op) => out.push(op.sign()),
            Token::Open => out.push('('),
            Token::Close => out.push(')'),
        }
    }
    out
}

/// 15 significant digits: 0.1 + 0.2 is 0.3, not 0.30000000000000004.
fn tidy(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    format!("{x:.14e}").parse().unwrap_or(x)
}

/// A number the way an entry holds it: `.` and a leading `-`, no exponent unless it is huge.
fn plain(x: f64) -> String {
    let x = if x == 0.0 { 0.0 } else { x };
    format!("{x}")
}

pub fn format(x: f64, sep: &Separators) -> String {
    let x = tidy(if x == 0.0 { 0.0 } else { x });
    let size = x.abs();
    if size != 0.0 && !(1e-12..1e16).contains(&size) {
        let text = format!("{x:.14e}");
        let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
        let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
        let exponent: i32 = exponent.parse().unwrap_or(0);
        return format!(
            "{}e{}{}",
            mantissa.replace('.', &sep.decimal).replace('-', "−"),
            if exponent < 0 { '−' } else { '+' },
            exponent.abs()
        );
    }
    group(&plain(x), sep)
}

/// An entry with its separators: `-1234.50` as `−1 234,50`.
fn group(entry: &str, sep: &Separators) -> String {
    let (negative, digits) = match entry.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, entry),
    };
    let (whole, fraction) = match digits.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (digits, None),
    };
    let mut out = String::new();
    if negative {
        out.push('−');
    }
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            out.push_str(&sep.group);
        }
        out.push(c);
    }
    if let Some(fraction) = fraction {
        out.push_str(&sep.decimal);
        out.push_str(fraction);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ru() -> Separators {
        Separators {
            decimal: ",".into(),
            group: " ".into(),
        }
    }

    fn en() -> Separators {
        Separators {
            decimal: ".".into(),
            group: ",".into(),
        }
    }

    /// Keys as on the page: digits, `.`, `+ - * /`, `=`, `%`, `n` for ±, `<` for ⌫, `c`, and the
    /// keyboard's brackets.
    fn typed(keys: &str) -> Engine {
        let mut e = Engine::default();
        for k in keys.chars().filter(|c| !c.is_whitespace()) {
            match k {
                '0'..='9' => e.digit(k as u8 - b'0'),
                '.' => e.point(),
                '+' => e.op(Op::Add),
                '-' => e.op(Op::Sub),
                '*' => e.op(Op::Mul),
                '/' => e.op(Op::Div),
                '=' => e.equals(),
                '%' => e.percent(),
                'n' => e.negate(),
                '<' => e.backspace(),
                'c' => e.clear(),
                '(' => e.open(),
                ')' => e.close(),
                _ => unreachable!("{k}"),
            }
        }
        e
    }

    fn shows(keys: &str) -> String {
        typed(keys).display(&en())
    }

    #[test]
    fn multiplication_binds_first() {
        assert_eq!(shows("2+3*4="), "14");
        assert_eq!(shows("10-6/3*2="), "6");
        assert_eq!(shows("1+2*3+4="), "11");
        assert_eq!(shows("8/2/2="), "2");
        assert_eq!(shows("10-2-3="), "5");
    }

    #[test]
    fn the_reverse_polish_order() {
        let tokens = typed("2+3*(4-1)").tokens;
        let rpn: Vec<String> = to_rpn(&tokens)
            .into_iter()
            .map(|t| match t {
                Token::Typed(n) => n.clone(),
                Token::Op(op) => op.sign().to_string(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(rpn, ["2", "3", "4", "1", "−", "×", "+"]);
    }

    #[test]
    fn brackets_count_first_and_close_by_themselves() {
        assert_eq!(shows("(2+3)*4="), "20");
        assert_eq!(shows("2*(3+4"), "4");
        assert_eq!(shows("2*(3+4="), "14");
        assert_eq!(shows("2(3+1)="), "8");
        assert_eq!(shows("(1+1)3="), "6");
        assert_eq!(typed("2*(3+4)").expression(&en()), "2 × (3 + 4)");
        assert_eq!(shows("(2+3)"), "5");
        assert_eq!(shows(")5"), "5", "nothing to close");
    }

    #[test]
    fn the_value_so_far_shows_after_an_operator() {
        assert_eq!(shows("2+3+"), "5");
        assert_eq!(shows("2+3*"), "3");
        assert_eq!(shows("2+3*4*"), "12");
        assert_eq!(shows("2*3+"), "6");
        assert_eq!(shows("2*(1+3+"), "4");
    }

    #[test]
    fn equals_again_repeats_the_last_operation() {
        assert_eq!(shows("2+3=="), "8");
        assert_eq!(shows("2*3==="), "54");
        assert_eq!(shows("10-1=7="), "6");
        assert_eq!(shows("2*(1+2)=="), "18");
    }

    #[test]
    fn an_operator_after_an_operator_replaces_it() {
        let e = typed("8+*2=");
        assert_eq!(e.display(&en()), "16");
        assert_eq!(e.expression(&en()), "8 × 2 =");
    }

    #[test]
    fn equals_without_a_second_number_takes_the_shown_one() {
        assert_eq!(shows("4*="), "16");
    }

    #[test]
    fn a_result_goes_on_with_an_operator_and_starts_over_with_a_digit() {
        assert_eq!(shows("2+3=*2="), "10");
        let e = typed("2+3=7");
        assert_eq!(e.display(&en()), "7");
        assert_eq!(e.expression(&en()), "");
    }

    #[test]
    fn the_line_over_the_number_tells_the_expression() {
        assert_eq!(typed("12+3*").expression(&en()), "12 + 3 ×");
        assert_eq!(typed("12+3*2=").expression(&en()), "12 + 3 × 2 =");
        assert_eq!(typed("1234.5+1").expression(&ru()), "1 234,5 +");
    }

    #[test]
    fn floating_point_noise_is_gone() {
        assert_eq!(shows(".1+.2="), "0.3");
        assert_eq!(shows("1/3*3="), "1");
        assert_eq!(shows("0.3-0.1="), "0.2");
    }

    #[test]
    fn division_by_zero_is_a_problem_until_the_next_key() {
        let mut e = typed("5/0=");
        assert_eq!(e.problem(), Some(Problem::DivideByZero));
        assert_eq!(e.copied(&en()), None);
        e.op(Op::Add);
        assert_eq!(e.problem(), Some(Problem::DivideByZero));
        e.digit(7);
        assert_eq!(e.problem(), None);
        assert_eq!(e.display(&en()), "7");
        assert_eq!(typed("0/0=").problem(), Some(Problem::Undefined));
        assert_eq!(typed("2/0=<").problem(), None);
        assert_eq!(typed("1/(2-2)=").problem(), Some(Problem::DivideByZero));
    }

    #[test]
    fn too_large_is_an_overflow() {
        let mut keys = "9999999999999999".to_string();
        for _ in 0..20 {
            keys.push_str("*9999999999999999");
        }
        keys.push('=');
        assert_eq!(typed(&keys).problem(), Some(Problem::Overflow));
    }

    #[test]
    fn percent_is_a_share_of_the_number_before() {
        assert_eq!(shows("200+10%="), "220");
        assert_eq!(shows("200-10%="), "180");
        assert_eq!(shows("50*10%="), "5");
        assert_eq!(shows("50%"), "0.5");
        assert_eq!(shows("200+10%"), "20");
        assert_eq!(typed("200+10%").expression(&en()), "200 +");
    }

    #[test]
    fn typing_keeps_what_was_typed() {
        assert_eq!(typed("0.50").display(&ru()), "0,50");
        assert_eq!(typed("1234567").display(&ru()), "1 234 567");
        assert_eq!(typed("1234567").display(&en()), "1,234,567");
        assert_eq!(shows("..5"), "0.5");
        assert_eq!(shows("007"), "7");
        assert_eq!(shows("12345678901234567890"), "1,234,567,890,123,456");
    }

    #[test]
    fn sign_and_backspace() {
        assert_eq!(shows("5n"), "−5");
        assert_eq!(shows("5nn"), "5");
        assert_eq!(shows("12<"), "1");
        assert_eq!(shows("5n<"), "0");
        assert_eq!(shows("2+3=<"), "5");
        assert_eq!(shows("2+3=n"), "−5");
        assert_eq!(shows("3-5="), "−2");
        assert_eq!(shows("2*n3="), "−6");
    }

    #[test]
    fn backspace_goes_back_through_the_expression() {
        let e = typed("12+3<<");
        assert_eq!(e.display(&en()), "12");
        assert_eq!(e.expression(&en()), "");
        assert_eq!(shows("12+3<<*2="), "24");
        assert_eq!(shows("2*(3<<"), "2");
    }

    #[test]
    fn copied_without_group_separators() {
        assert_eq!(typed("1234.5").copied(&ru()).as_deref(), Some("1234,5"));
        assert_eq!(
            typed("1000*1000=").copied(&en()).as_deref(),
            Some("1000000")
        );
    }

    #[test]
    fn huge_and_tiny_numbers_take_an_exponent() {
        assert_eq!(format(1.5e20, &ru()), "1,5e+20");
        assert_eq!(format(-2e-15, &en()), "−2e−15");
        assert_eq!(format(1e15, &en()), "1,000,000,000,000,000");
        assert_eq!(format(-0.0, &en()), "0");
    }

    #[test]
    fn clear_starts_over() {
        let e = typed("2+3c");
        assert_eq!(e, Engine::default());
    }
}
