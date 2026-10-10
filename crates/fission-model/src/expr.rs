//! Typed values: numbers with units and simple arithmetic, like Fusion's
//! value boxes (`10+5`, `1 in`, `3*2.5mm`, `(20-4)/2`). Lengths come out in
//! mm, angles in degrees.

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Quantity {
    Length,
    Angle,
    Count,
}

/// Evaluate `text` as a quantity. A bare number takes the default unit (mm
/// or degrees).
pub fn evaluate(text: &str, quantity: Quantity) -> Result<f64, String> {
    evaluate_with(text, quantity, &|_| None)
}

/// Like [`evaluate`], with named values (user parameters, already in mm or
/// degrees) that `lookup` resolves.
pub fn evaluate_with(text: &str, quantity: Quantity, lookup: &dyn Fn(&str) -> Option<f64>) -> Result<f64, String> {
    let mut p = Parser { s: text.trim().as_bytes(), i: 0, quantity, lookup };
    if p.s.is_empty() {
        return Err("enter a value".into());
    }
    let v = p.expr()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(format!("unexpected '{}'", &text.trim()[p.i..]));
    }
    if !v.is_finite() {
        return Err("value isn't a finite number".into());
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    quantity: Quantity,
    lookup: &'a dyn Fn(&str) -> Option<f64>,
}

/// The names an expression refers to (not units, which follow numbers).
pub fn names_in(text: &str) -> Vec<String> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut after_value = false;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_digit() || c == b'.' {
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.' || b[i] == b',') {
                i += 1;
            }
            after_value = true;
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            if !after_value {
                out.push(text[start..i].to_string());
            }
            after_value = true;
            continue;
        }
        if !c.is_ascii_whitespace() {
            after_value = c == b')';
        }
        i += 1;
    }
    out
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.s.get(self.i).is_some_and(|c| c.is_ascii_whitespace()) {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }

    fn expr(&mut self) -> Result<f64, String> {
        let mut v = self.term()?;
        while let Some(op @ (b'+' | b'-')) = self.peek() {
            self.i += 1;
            let r = self.term()?;
            v = if op == b'+' { v + r } else { v - r };
        }
        Ok(v)
    }

    fn term(&mut self) -> Result<f64, String> {
        let mut v = self.factor()?;
        while let Some(op @ (b'*' | b'/')) = self.peek() {
            self.i += 1;
            let r = self.factor()?;
            if op == b'/' && r == 0.0 {
                return Err("division by zero".into());
            }
            v = if op == b'*' { v * r } else { v / r };
        }
        Ok(v)
    }

    fn factor(&mut self) -> Result<f64, String> {
        match self.peek() {
            Some(b'-') => {
                self.i += 1;
                Ok(-self.factor()?)
            }
            Some(b'+') => {
                self.i += 1;
                self.factor()
            }
            Some(b'(') => {
                self.i += 1;
                let v = self.expr()?;
                if self.peek() != Some(b')') {
                    return Err("missing ')'".into());
                }
                self.i += 1;
                Ok(v * self.unit()?)
            }
            Some(c) if c.is_ascii_digit() || c == b'.' || c == b',' => {
                let start = self.i;
                while self.s.get(self.i).is_some_and(|c| c.is_ascii_digit() || *c == b'.' || *c == b',') {
                    self.i += 1;
                }
                // Accept a decimal comma too.
                let text = std::str::from_utf8(&self.s[start..self.i]).unwrap().replace(',', ".");
                let n: f64 = text.parse().map_err(|_| format!("'{text}' isn't a number"))?;
                Ok(n * self.unit()?)
            }
            // A parameter name (units only follow numbers).
            Some(c) if c.is_ascii_alphabetic() || c == b'_' => {
                let start = self.i;
                while self.s.get(self.i).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_') {
                    self.i += 1;
                }
                let name = std::str::from_utf8(&self.s[start..self.i]).unwrap();
                (self.lookup)(name).ok_or_else(|| format!("no parameter named '{name}'"))
            }
            Some(c) => Err(format!("unexpected '{}'", c as char)),
            None => Err("value is incomplete".into()),
        }
    }

    /// Optional unit after a number; returns the factor to mm or degrees.
    fn unit(&mut self) -> Result<f64, String> {
        self.ws();
        let start = self.i;
        while self.s.get(self.i).is_some_and(|c| c.is_ascii_alphabetic() || *c == b'"' || *c == b'\'') {
            self.i += 1;
        }
        let unit = std::str::from_utf8(&self.s[start..self.i]).unwrap().to_ascii_lowercase();
        let length = |f: f64| match self.quantity {
            Quantity::Length => Ok(f),
            _ => Err(format!("'{unit}' is a length unit")),
        };
        match unit.as_str() {
            "" => Ok(1.0),
            "mm" => length(1.0),
            "cm" => length(10.0),
            "m" => length(1000.0),
            "um" => length(0.001),
            "in" | "\"" => length(25.4),
            "ft" | "'" => length(304.8),
            "deg" | "d" if self.quantity == Quantity::Angle => Ok(1.0),
            "rad" if self.quantity == Quantity::Angle => Ok(180.0 / std::f64::consts::PI),
            other => Err(format!("unknown unit '{other}'")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn len(s: &str) -> f64 {
        evaluate(s, Quantity::Length).unwrap()
    }

    #[test]
    fn numbers_units_and_arithmetic() {
        assert_eq!(len("12"), 12.0);
        assert_eq!(len("12.5 mm"), 12.5);
        assert_eq!(len("1in"), 25.4);
        assert_eq!(len("2 cm + 5"), 25.0);
        assert_eq!(len("3*2.5mm"), 7.5);
        assert_eq!(len("(20-4)/2"), 8.0);
        assert_eq!(len("-5"), -5.0);
        assert_eq!(len("1,5"), 1.5);
        assert_eq!(evaluate("90 deg", Quantity::Angle).unwrap(), 90.0);
        assert!((evaluate("1 rad", Quantity::Angle).unwrap() - 57.29578).abs() < 1e-4);
    }

    #[test]
    fn parameters_by_name() {
        let lookup = |n: &str| match n {
            "width" => Some(40.0),
            "wall" => Some(2.0),
            _ => None,
        };
        assert_eq!(evaluate_with("width", Quantity::Length, &lookup).unwrap(), 40.0);
        assert_eq!(evaluate_with("width/2 - wall", Quantity::Length, &lookup).unwrap(), 18.0);
        assert_eq!(evaluate_with("(width + 1 in)", Quantity::Length, &lookup).unwrap(), 65.4);
        assert!(evaluate_with("depth", Quantity::Length, &lookup).unwrap_err().contains("depth"));
        assert_eq!(names_in("width/2 - wall + 3 mm"), vec!["width", "wall"]);
        assert!(names_in("25 mm").is_empty());
    }

    #[test]
    fn errors_are_explained() {
        assert!(evaluate("", Quantity::Length).is_err());
        assert!(evaluate("5 parsecs", Quantity::Length).is_err());
        assert!(evaluate("5 in", Quantity::Angle).is_err());
        assert!(evaluate("4/0", Quantity::Length).is_err());
        assert!(evaluate("(3+", Quantity::Length).is_err());
        assert!(evaluate("3 3", Quantity::Length).is_err());
    }
}
