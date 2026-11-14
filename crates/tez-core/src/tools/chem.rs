//! Công cụ hóa học dùng chung: parser công thức, khối lượng mol, bảng nguyên tố.

use std::collections::BTreeMap;

pub type Composition = BTreeMap<String, i64>;

/// Khối lượng nguyên tử tương đối (g/mol), làm tròn theo IUPAC.
pub fn atomic_mass(symbol: &str) -> Option<f64> {
    let mass = match symbol {
        "H" => 1.008,
        "He" => 4.0026,
        "Li" => 6.94,
        "Be" => 9.0122,
        "B" => 10.81,
        "C" => 12.011,
        "N" => 14.007,
        "O" => 15.999,
        "F" => 18.998,
        "Ne" => 20.180,
        "Na" => 22.990,
        "Mg" => 24.305,
        "Al" => 26.982,
        "Si" => 28.085,
        "P" => 30.974,
        "S" => 32.06,
        "Cl" => 35.45,
        "Ar" => 39.948,
        "K" => 39.098,
        "Ca" => 40.078,
        "Sc" => 44.956,
        "Ti" => 47.867,
        "V" => 50.942,
        "Cr" => 51.996,
        "Mn" => 54.938,
        "Fe" => 55.845,
        "Co" => 58.933,
        "Ni" => 58.693,
        "Cu" => 63.546,
        "Zn" => 65.38,
        "Ga" => 69.723,
        "Ge" => 72.630,
        "As" => 74.922,
        "Se" => 78.971,
        "Br" => 79.904,
        "Kr" => 83.798,
        "Rb" => 85.468,
        "Sr" => 87.62,
        "Y" => 88.906,
        "Zr" => 91.224,
        "Nb" => 92.906,
        "Mo" => 95.95,
        "Tc" => 98.0,
        "Ru" => 101.07,
        "Rh" => 102.91,
        "Pd" => 106.42,
        "Ag" => 107.87,
        "Cd" => 112.41,
        "In" => 114.82,
        "Sn" => 118.71,
        "Sb" => 121.76,
        "Te" => 127.60,
        "I" => 126.90,
        "Xe" => 131.29,
        "Cs" => 132.91,
        "Ba" => 137.33,
        "La" => 138.91,
        "Ce" => 140.12,
        "Pr" => 140.91,
        "Nd" => 144.24,
        "Pm" => 145.0,
        "Sm" => 150.36,
        "Eu" => 151.96,
        "Gd" => 157.25,
        "Tb" => 158.93,
        "Dy" => 162.50,
        "Ho" => 164.93,
        "Er" => 167.26,
        "Tm" => 168.93,
        "Yb" => 173.05,
        "Lu" => 174.97,
        "Hf" => 178.49,
        "Ta" => 180.95,
        "W" => 183.84,
        "Re" => 186.21,
        "Os" => 190.23,
        "Ir" => 192.22,
        "Pt" => 195.08,
        "Au" => 196.97,
        "Hg" => 200.59,
        "Tl" => 204.38,
        "Pb" => 207.2,
        "Bi" => 208.98,
        "Po" => 209.0,
        "At" => 210.0,
        "Rn" => 222.0,
        "Fr" => 223.0,
        "Ra" => 226.0,
        "Ac" => 227.0,
        "Th" => 232.04,
        "Pa" => 231.04,
        "U" => 238.03,
        _ => return None,
    };
    Some(mass)
}

/// Tách hệ số đứng trước (ví dụ `2H2O` → `(2, "H2O")`). Mặc định 1.
fn split_leading_number(part: &str) -> (i64, &str) {
    let digit_end = part
        .char_indices()
        .take_while(|(_, c)| c.is_ascii_digit())
        .map(|(i, c)| i + c.len_utf8())
        .last()
        .unwrap_or(0);
    if digit_end == 0 {
        return (1, part);
    }
    match part[..digit_end].parse::<i64>() {
        Ok(n) if n > 0 => (n, &part[digit_end..]),
        _ => (1, part),
    }
}

fn parse_number(chars: &[char], pos: &mut usize) -> i64 {
    let start = *pos;
    while *pos < chars.len() && chars[*pos].is_ascii_digit() {
        *pos += 1;
    }
    if start == *pos {
        return 1;
    }
    chars[start..*pos]
        .iter()
        .collect::<String>()
        .parse::<i64>()
        .unwrap_or(1)
}

fn parse_group(chars: &[char], pos: &mut usize, out: &mut Composition) -> Result<(), String> {
    while *pos < chars.len() {
        let c = chars[*pos];
        match c {
            ')' => return Ok(()),
            '(' => {
                *pos += 1;
                let mut inner = Composition::new();
                parse_group(chars, pos, &mut inner)?;
                if *pos >= chars.len() || chars[*pos] != ')' {
                    return Err("thiếu dấu ')' trong công thức".to_string());
                }
                *pos += 1;
                let multiplier = parse_number(chars, pos);
                for (element, count) in inner {
                    *out.entry(element).or_insert(0) += count * multiplier;
                }
            }
            c if c.is_ascii_uppercase() => {
                let mut symbol = c.to_string();
                *pos += 1;
                if *pos < chars.len() && chars[*pos].is_ascii_lowercase() {
                    symbol.push(chars[*pos]);
                    *pos += 1;
                }
                if atomic_mass(&symbol).is_none() {
                    return Err(format!("nguyên tố không biết: {symbol}"));
                }
                let count = parse_number(chars, pos);
                *out.entry(symbol).or_insert(0) += count;
            }
            c => return Err(format!("ký tự không hợp lệ trong công thức: '{c}'")),
        }
    }
    Ok(())
}

/// Parse công thức hóa học thành số nguyên tử từng nguyên tố.
///
/// Hỗ trợ ngoặc đơn và hydrate: `Ca(OH)2`, `Fe2(SO4)3`, `CuSO4·5H2O`.
pub fn parse_formula(formula: &str) -> Result<Composition, String> {
    let cleaned: String = formula.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.is_empty() {
        return Err("công thức rỗng".to_string());
    }
    super::check_input_bounds(&cleaned)?;

    let mut total = Composition::new();
    for part in cleaned.split(['·', '*']) {
        if part.is_empty() {
            return Err(format!("công thức không hợp lệ: '{formula}'"));
        }
        let (multiplier, rest) = split_leading_number(part);
        let chars: Vec<char> = rest.chars().collect();
        let mut counts = Composition::new();
        let mut pos = 0;
        parse_group(&chars, &mut pos, &mut counts)?;
        if pos != chars.len() {
            return Err(format!("ký tự thừa trong '{part}'"));
        }
        for (element, count) in counts {
            *total.entry(element).or_insert(0) += count * multiplier;
        }
    }
    Ok(total)
}

/// Khối lượng mol (g/mol) từ thành phần nguyên tố.
pub fn molar_mass(composition: &Composition) -> Result<f64, String> {
    let mut total = 0.0;
    for (element, count) in composition {
        let mass =
            atomic_mass(element).ok_or_else(|| format!("nguyên tố không biết: {element}"))?;
        total += mass * (*count as f64);
    }
    Ok(total)
}

/// In thành phần theo dạng `H2O`, `Ca(OH)2` không tái tạo được ngoặc.
pub fn format_composition(composition: &Composition) -> String {
    let mut out = String::new();
    for (element, count) in composition {
        out.push_str(element);
        if *count != 1 {
            out.push_str(&count.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 0.01, "{a} != {b}");
    }

    #[test]
    fn parses_simple_formula() {
        let c = parse_formula("H2O").unwrap();
        assert_eq!(c["H"], 2);
        assert_eq!(c["O"], 1);
        approx(molar_mass(&c).unwrap(), 18.015);
    }

    #[test]
    fn parses_parentheses() {
        let c = parse_formula("Ca(OH)2").unwrap();
        assert_eq!(c["Ca"], 1);
        assert_eq!(c["O"], 2);
        assert_eq!(c["H"], 2);
        approx(molar_mass(&c).unwrap(), 74.094);
    }

    #[test]
    fn parses_nested_and_hydrate() {
        let c = parse_formula("CuSO4·5H2O").unwrap();
        assert_eq!(c["Cu"], 1);
        assert_eq!(c["S"], 1);
        assert_eq!(c["O"], 9);
        assert_eq!(c["H"], 10);
        approx(molar_mass(&c).unwrap(), 249.677);
    }

    #[test]
    fn rejects_unknown_element() {
        assert!(parse_formula("Xx2O").is_err());
    }

    #[test]
    fn rejects_deeply_nested_formula() {
        let deep = format!("{}H{}", "(".repeat(5000), ")".repeat(5000));
        assert!(parse_formula(&deep).is_err());
    }
}
