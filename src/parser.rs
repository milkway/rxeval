//! XPath 1.0 expression parser.
//!
//! The grammar is the one in the XPath 1.0 recommendation, with the
//! precedence climbing written out rather than generated, so an unexpected
//! token names itself and its position.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Name(String),
    Number(f64),
    Literal(String),
    /// `$name`
    Variable(String),
    Slash,
    DoubleSlash,
    Dot,
    DoubleDot,
    At,
    Star,
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    Comma,
    Pipe,
    Plus,
    Minus,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    /// `::`
    DoubleColon,
}

pub fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut out = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            c if c.is_whitespace() => i += 1,
            '/' => {
                if chars.get(i + 1) == Some(&'/') {
                    out.push(Token::DoubleSlash);
                    i += 2;
                } else {
                    out.push(Token::Slash);
                    i += 1;
                }
            }
            '.' => {
                if chars.get(i + 1) == Some(&'.') {
                    out.push(Token::DoubleDot);
                    i += 2;
                } else if chars.get(i + 1).is_some_and(|c| c.is_ascii_digit()) {
                    let start = i;
                    i += 1;
                    while chars.get(i).is_some_and(|c| c.is_ascii_digit()) {
                        i += 1;
                    }
                    let text: String = chars[start..i].iter().collect();
                    out.push(Token::Number(
                        text.parse()
                            .map_err(|_| format!("'{text}' is not a number"))?,
                    ));
                } else {
                    out.push(Token::Dot);
                    i += 1;
                }
            }
            '@' => {
                out.push(Token::At);
                i += 1;
            }
            '*' => {
                out.push(Token::Star);
                i += 1;
            }
            '(' => {
                out.push(Token::LeftParen);
                i += 1;
            }
            ')' => {
                out.push(Token::RightParen);
                i += 1;
            }
            '[' => {
                out.push(Token::LeftBracket);
                i += 1;
            }
            ']' => {
                out.push(Token::RightBracket);
                i += 1;
            }
            ',' => {
                out.push(Token::Comma);
                i += 1;
            }
            '|' => {
                out.push(Token::Pipe);
                i += 1;
            }
            '+' => {
                out.push(Token::Plus);
                i += 1;
            }
            '-' => {
                out.push(Token::Minus);
                i += 1;
            }
            '=' => {
                out.push(Token::Equal);
                i += 1;
            }
            '!' if chars.get(i + 1) == Some(&'=') => {
                out.push(Token::NotEqual);
                i += 2;
            }
            '<' => {
                if chars.get(i + 1) == Some(&'=') {
                    out.push(Token::LessEqual);
                    i += 2;
                } else {
                    out.push(Token::Less);
                    i += 1;
                }
            }
            '>' => {
                if chars.get(i + 1) == Some(&'=') {
                    out.push(Token::GreaterEqual);
                    i += 2;
                } else {
                    out.push(Token::Greater);
                    i += 1;
                }
            }
            ':' if chars.get(i + 1) == Some(&':') => {
                out.push(Token::DoubleColon);
                i += 2;
            }
            '\'' | '"' => {
                let quote = c;
                i += 1;
                let start = i;
                while i < chars.len() && chars[i] != quote {
                    i += 1;
                }
                if i >= chars.len() {
                    return Err(format!("unterminated string starting at character {start}"));
                }
                out.push(Token::Literal(chars[start..i].iter().collect()));
                i += 1;
            }
            '$' => {
                i += 1;
                let start = i;
                while i < chars.len() && is_name_char(chars[i]) {
                    i += 1;
                }
                out.push(Token::Variable(chars[start..i].iter().collect()));
            }
            c if c.is_ascii_digit() => {
                let start = i;
                while chars
                    .get(i)
                    .is_some_and(|c| c.is_ascii_digit() || *c == '.')
                {
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                out.push(Token::Number(
                    text.parse()
                        .map_err(|_| format!("'{text}' is not a number"))?,
                ));
            }
            c if is_name_start(c) => {
                let start = i;
                // a QName keeps its prefix: jr:choice-name and instance()
                // are told apart by it
                while i < chars.len() && (is_name_char(chars[i]) || chars[i] == ':') {
                    // but `::` is an axis separator, not part of the name
                    if chars[i] == ':' && chars.get(i + 1) == Some(&':') {
                        break;
                    }
                    i += 1;
                }
                out.push(Token::Name(chars[start..i].iter().collect()));
            }
            other => return Err(format!("unexpected character '{other}'")),
        }
    }
    Ok(out)
}

fn is_name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '.'
}

// ---------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Axis {
    Child,
    Parent,
    Self_,
    Descendant,
    DescendantOrSelf,
    Ancestor,
    AncestorOrSelf,
    Attribute,
    Following,
    FollowingSibling,
    Preceding,
    PrecedingSibling,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NameTest {
    Any,
    Named(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub axis: Axis,
    pub test: NameTest,
    pub predicates: Vec<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f64),
    Literal(String),
    Variable(String),
    /// `/a/b` when absolute, `a/b` when not.
    Path {
        absolute: bool,
        steps: Vec<Step>,
    },
    /// A path rooted at another expression: `instance('x')/root/item`.
    Filter {
        base: Box<Expr>,
        predicates: Vec<Expr>,
        steps: Vec<Step>,
    },
    Function {
        name: String,
        args: Vec<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Negate(Box<Expr>),
    Union(Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinaryOp {
    Or,
    And,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
}

impl fmt::Display for BinaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            BinaryOp::Or => "or",
            BinaryOp::And => "and",
            BinaryOp::Equal => "=",
            BinaryOp::NotEqual => "!=",
            BinaryOp::Less => "<",
            BinaryOp::LessEqual => "<=",
            BinaryOp::Greater => ">",
            BinaryOp::GreaterEqual => ">=",
            BinaryOp::Add => "+",
            BinaryOp::Subtract => "-",
            BinaryOp::Multiply => "*",
            BinaryOp::Divide => "div",
            BinaryOp::Modulo => "mod",
        };
        write!(f, "{text}")
    }
}

pub fn parse(expression: &str) -> Result<Expr, String> {
    let tokens = tokenize(expression)?;
    let mut parser = Parser { tokens, pos: 0 };
    let expr = parser.parse_or()?;
    if parser.pos < parser.tokens.len() {
        return Err(format!(
            "unexpected trailing input in {expression:?} at token {}",
            parser.pos
        ));
    }
    Ok(expr)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn eat(&mut self, token: &Token) -> bool {
        if self.peek() == Some(token) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, token: &Token) -> Result<(), String> {
        if self.eat(token) {
            Ok(())
        } else {
            Err(format!("expected {token:?}, found {:?}", self.peek()))
        }
    }

    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_and()?;
        while self.peek() == Some(&Token::Name("or".into())) {
            self.pos += 1;
            let right = self.parse_and()?;
            left = Expr::Binary {
                op: BinaryOp::Or,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_equality()?;
        while self.peek() == Some(&Token::Name("and".into())) {
            self.pos += 1;
            let right = self.parse_equality()?;
            left = Expr::Binary {
                op: BinaryOp::And,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_relational()?;
        loop {
            let op = match self.peek() {
                Some(Token::Equal) => BinaryOp::Equal,
                Some(Token::NotEqual) => BinaryOp::NotEqual,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_relational()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_relational(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_additive()?;
        loop {
            let op = match self.peek() {
                Some(Token::Less) => BinaryOp::Less,
                Some(Token::LessEqual) => BinaryOp::LessEqual,
                Some(Token::Greater) => BinaryOp::Greater,
                Some(Token::GreaterEqual) => BinaryOp::GreaterEqual,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_additive()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let op = match self.peek() {
                Some(Token::Plus) => BinaryOp::Add,
                Some(Token::Minus) => BinaryOp::Subtract,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_multiplicative()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Some(Token::Star) => BinaryOp::Multiply,
                Some(Token::Name(n)) if n == "div" => BinaryOp::Divide,
                Some(Token::Name(n)) if n == "mod" => BinaryOp::Modulo,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_unary()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        if self.eat(&Token::Minus) {
            return Ok(Expr::Negate(Box::new(self.parse_unary()?)));
        }
        self.parse_union()
    }

    fn parse_union(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_path()?;
        while self.eat(&Token::Pipe) {
            let right = self.parse_path()?;
            left = Expr::Union(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_path(&mut self) -> Result<Expr, String> {
        // absolute path
        if self.eat(&Token::DoubleSlash) {
            let mut steps = vec![Step {
                axis: Axis::DescendantOrSelf,
                test: NameTest::Any,
                predicates: Vec::new(),
            }];
            steps.extend(self.parse_relative_steps()?);
            return Ok(Expr::Path {
                absolute: true,
                steps,
            });
        }
        if self.eat(&Token::Slash) {
            if self.at_step_start() {
                return Ok(Expr::Path {
                    absolute: true,
                    steps: self.parse_relative_steps()?,
                });
            }
            return Ok(Expr::Path {
                absolute: true,
                steps: Vec::new(),
            });
        }

        // a primary expression may begin a path: instance('x')/a/b
        if let Some(base) = self.try_parse_primary()? {
            let mut predicates = Vec::new();
            while self.eat(&Token::LeftBracket) {
                predicates.push(self.parse_or()?);
                self.expect(&Token::RightBracket)?;
            }
            let mut steps = Vec::new();
            loop {
                if self.eat(&Token::DoubleSlash) {
                    steps.push(Step {
                        axis: Axis::DescendantOrSelf,
                        test: NameTest::Any,
                        predicates: Vec::new(),
                    });
                    steps.extend(self.parse_relative_steps()?);
                } else if self.eat(&Token::Slash) {
                    steps.extend(self.parse_relative_steps()?);
                } else {
                    break;
                }
            }
            if predicates.is_empty() && steps.is_empty() {
                return Ok(base);
            }
            return Ok(Expr::Filter {
                base: Box::new(base),
                predicates,
                steps,
            });
        }

        Ok(Expr::Path {
            absolute: false,
            steps: self.parse_relative_steps()?,
        })
    }

    fn at_step_start(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token::Name(_))
                | Some(Token::Star)
                | Some(Token::At)
                | Some(Token::Dot)
                | Some(Token::DoubleDot)
        )
    }

    /// A primary expression, when the next tokens are one: a literal, a
    /// number, a variable, a parenthesised expression, or a function call.
    fn try_parse_primary(&mut self) -> Result<Option<Expr>, String> {
        match self.peek().cloned() {
            Some(Token::Number(n)) => {
                self.pos += 1;
                Ok(Some(Expr::Number(n)))
            }
            Some(Token::Literal(s)) => {
                self.pos += 1;
                Ok(Some(Expr::Literal(s)))
            }
            Some(Token::Variable(v)) => {
                self.pos += 1;
                Ok(Some(Expr::Variable(v)))
            }
            Some(Token::LeftParen) => {
                self.pos += 1;
                let inner = self.parse_or()?;
                self.expect(&Token::RightParen)?;
                Ok(Some(inner))
            }
            // A name followed by `(` is a function call — unless it is a node
            // type test, which XPath spells the same way.
            Some(Token::Name(name))
                if self.tokens.get(self.pos + 1) == Some(&Token::LeftParen)
                    && !is_node_type(&name) =>
            {
                self.pos += 2;
                let mut args = Vec::new();
                if !self.eat(&Token::RightParen) {
                    loop {
                        args.push(self.parse_or()?);
                        if self.eat(&Token::Comma) {
                            continue;
                        }
                        self.expect(&Token::RightParen)?;
                        break;
                    }
                }
                Ok(Some(Expr::Function { name, args }))
            }
            _ => Ok(None),
        }
    }

    fn parse_relative_steps(&mut self) -> Result<Vec<Step>, String> {
        let mut steps = vec![self.parse_step()?];
        loop {
            if self.eat(&Token::DoubleSlash) {
                steps.push(Step {
                    axis: Axis::DescendantOrSelf,
                    test: NameTest::Any,
                    predicates: Vec::new(),
                });
                steps.push(self.parse_step()?);
            } else if self.eat(&Token::Slash) {
                steps.push(self.parse_step()?);
            } else {
                break;
            }
        }
        Ok(steps)
    }

    fn parse_step(&mut self) -> Result<Step, String> {
        if self.eat(&Token::Dot) {
            return Ok(Step {
                axis: Axis::Self_,
                test: NameTest::Any,
                predicates: self.parse_predicates()?,
            });
        }
        if self.eat(&Token::DoubleDot) {
            return Ok(Step {
                axis: Axis::Parent,
                test: NameTest::Any,
                predicates: self.parse_predicates()?,
            });
        }
        let mut axis = Axis::Child;
        if self.eat(&Token::At) {
            axis = Axis::Attribute;
        } else if let Some(Token::Name(name)) = self.peek().cloned() {
            if self.tokens.get(self.pos + 1) == Some(&Token::DoubleColon) {
                axis = named_axis(&name)?;
                self.pos += 2;
            }
        }
        let test = match self.peek().cloned() {
            Some(Token::Star) => {
                self.pos += 1;
                NameTest::Any
            }
            Some(Token::Name(name)) => {
                self.pos += 1;
                // node type tests reach here as a name followed by ()
                if is_node_type(&name) && self.eat(&Token::LeftParen) {
                    self.expect(&Token::RightParen)?;
                    NameTest::Any
                } else {
                    NameTest::Named(strip_prefix(&name))
                }
            }
            other => return Err(format!("expected a node test, found {other:?}")),
        };
        Ok(Step {
            axis,
            test,
            predicates: self.parse_predicates()?,
        })
    }

    fn parse_predicates(&mut self) -> Result<Vec<Expr>, String> {
        let mut out = Vec::new();
        while self.eat(&Token::LeftBracket) {
            out.push(self.parse_or()?);
            self.expect(&Token::RightBracket)?;
        }
        Ok(out)
    }
}

fn is_node_type(name: &str) -> bool {
    matches!(name, "node" | "text" | "comment" | "processing-instruction")
}

fn strip_prefix(qname: &str) -> String {
    match qname.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => qname.to_string(),
    }
}

fn named_axis(name: &str) -> Result<Axis, String> {
    Ok(match name {
        "child" => Axis::Child,
        "parent" => Axis::Parent,
        "self" => Axis::Self_,
        "descendant" => Axis::Descendant,
        "descendant-or-self" => Axis::DescendantOrSelf,
        "ancestor" => Axis::Ancestor,
        "ancestor-or-self" => Axis::AncestorOrSelf,
        "attribute" => Axis::Attribute,
        "following" => Axis::Following,
        "following-sibling" => Axis::FollowingSibling,
        "preceding" => Axis::Preceding,
        "preceding-sibling" => Axis::PrecedingSibling,
        // namespace:: is the one axis instances never use, and guessing at
        // it would be worse than saying so
        other => return Err(format!("unsupported axis '{other}'")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(expression: &str) -> Expr {
        parse(expression).unwrap_or_else(|e| panic!("{expression:?}: {e}"))
    }

    #[test]
    fn parses_the_shapes_real_forms_use() {
        round_trip("/data/age");
        round_trip("../name");
        round_trip("selected(../services, 'water')");
        round_trip("/data/age >= 18 and /data/consent = 'yes'");
        round_trip("count(/data/resident) > 0");
        round_trip("instance('lugares')/root/item[name = /data/lugar]/label");
        round_trip("if(/data/a = 1, 'one', 'other')");
        round_trip("../resident[position() = 2]/name");
        round_trip("//age");
        round_trip("@id");
        round_trip("-3 + 4 * 2 div 2 mod 3");
        round_trip("concat('a', 'b', 'c')");
        round_trip("/data/a | /data/b");
    }

    #[test]
    fn precedence_binds_the_way_xpath_says() {
        // `or` is loosest, then `and`, then comparison, then arithmetic
        let expr = parse("1 + 2 = 3 and true()").unwrap();
        match expr {
            Expr::Binary {
                op: BinaryOp::And,
                left,
                ..
            } => match *left {
                Expr::Binary {
                    op: BinaryOp::Equal,
                    left,
                    ..
                } => assert!(matches!(
                    *left,
                    Expr::Binary {
                        op: BinaryOp::Add,
                        ..
                    }
                )),
                other => panic!("expected an equality below the and, got {other:?}"),
            },
            other => panic!("expected an and at the top, got {other:?}"),
        }
    }

    #[test]
    fn a_bad_expression_says_what_it_saw() {
        assert!(parse("/data/age >").is_err());
        assert!(parse("concat('a'").is_err());
        assert!(parse("'unterminated").is_err());
        assert!(parse("namespace::x").is_err());
    }
}
