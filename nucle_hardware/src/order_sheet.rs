//! Oligo-pool order sheets: the strands a program stores, in a file a
//! vendor's ordering page accepts, checked against that vendor's published
//! limits. Writing one orders nothing; a person uploads it.
//!
//! The limits are the ones the vendors publish: IDT's oPools entry page
//! (Excel uploads only, 40-350 bases, 2-384 oligos at 50 pmol, 10-2,000 at
//! 10 pmol, 100-20,000 at 1 pmol) and Twist's oligo-pool ordering
//! requirements as university core facilities pass them on (a .txt, .csv
//! or .fasta file with one oligo per line, 20-350 nt). IDT does not publish
//! its template's column names, so its sheet is a two-column CSV to paste
//! into the template its page provides.

use std::collections::HashSet;

/// One oligo to synthesize.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Oligo {
    pub name: String,
    pub sequence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Twist,
    Idt,
}

/// The file an order sheet is written as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheetFormat {
    /// `>name` lines, each followed by its sequence.
    Fasta,
    /// A `Name,Sequence` header and one oligo per row.
    Csv,
}

/// IDT oPools scales: amount per oligo, and the fewest and most oligos a
/// pool at that scale may hold.
pub const IDT_SCALES: [(&str, usize, usize); 3] = [("50 pmol", 2, 384), ("10 pmol", 10, 2_000), ("1 pmol", 100, 20_000)];

impl Vendor {
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "twist" => Some(Vendor::Twist),
            "idt" => Some(Vendor::Idt),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Vendor::Twist => "Twist",
            Vendor::Idt => "IDT",
        }
    }

    /// Shortest and longest oligo the vendor makes in a pool, in nucleotides.
    pub fn length_range(self) -> (usize, usize) {
        match self {
            Vendor::Twist => (20, 350),
            Vendor::Idt => (40, 350),
        }
    }

    /// The sheet format for an output path, by its extension, or why this
    /// vendor cannot take it.
    pub fn format_for(self, path: &str) -> Result<SheetFormat, String> {
        let extension = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        match (self, extension.as_str()) {
            (Vendor::Twist, "fasta" | "fa") => Ok(SheetFormat::Fasta),
            (_, "csv") => Ok(SheetFormat::Csv),
            (Vendor::Idt, _) => Err("IDT's oPools page takes Excel uploads only; write a .csv and paste its Name and Sequence columns into the template the page provides".to_string()),
            (Vendor::Twist, _) => Err("Twist takes a .fasta or .csv sequence file; end the path with one of those".to_string()),
        }
    }
}

/// What a vendor would make of a set of oligos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderCheck {
    pub oligos: usize,
    pub shortest: usize,
    pub longest: usize,
    /// IDT scales whose pool size fits the order; empty for Twist.
    pub idt_scales: Vec<&'static str>,
    /// Everything the vendor would refuse. Empty when the order is fine.
    pub problems: Vec<String>,
}

/// Checks `oligos` against `vendor`'s limits: oligo lengths, plain
/// A/C/G/T sequences, unique names, and for IDT a pool size some scale
/// accepts.
pub fn check(vendor: Vendor, oligos: &[Oligo]) -> OrderCheck {
    let (min, max) = vendor.length_range();
    let mut problems = Vec::new();
    if oligos.is_empty() {
        problems.push("the program stores nothing to synthesize".to_string());
    }
    let mut names = HashSet::new();
    for oligo in oligos {
        let len = oligo.sequence.len();
        if len < min || len > max {
            problems.push(format!("{} is {} nt; {} makes {}-{} nt oligos", oligo.name, len, vendor.name(), min, max));
        }
        if let Some(bad) = oligo.sequence.chars().find(|c| !matches!(c, 'A' | 'C' | 'G' | 'T')) {
            problems.push(format!("{} contains '{}'; only A, C, G and T are ordered", oligo.name, bad));
        }
        if oligo.name.is_empty() || !names.insert(oligo.name.as_str()) {
            problems.push(format!("oligo name '{}' is empty or used twice", oligo.name));
        }
    }
    let idt_scales: Vec<&'static str> = match vendor {
        Vendor::Idt => IDT_SCALES.iter().filter(|(_, lo, hi)| (*lo..=*hi).contains(&oligos.len())).map(|(scale, _, _)| *scale).collect(),
        Vendor::Twist => Vec::new(),
    };
    if vendor == Vendor::Idt && idt_scales.is_empty() && !oligos.is_empty() {
        problems.push(format!("an IDT oPools pool holds 2 to 20,000 oligos; this order has {}", oligos.len()));
    }
    OrderCheck {
        oligos: oligos.len(),
        shortest: oligos.iter().map(|o| o.sequence.len()).min().unwrap_or(0),
        longest: oligos.iter().map(|o| o.sequence.len()).max().unwrap_or(0),
        idt_scales,
        problems,
    }
}

/// The sheet's text.
pub fn render(format: SheetFormat, oligos: &[Oligo]) -> String {
    match format {
        SheetFormat::Fasta => oligos.iter().map(|o| format!(">{}\n{}\n", o.name, o.sequence)).collect(),
        SheetFormat::Csv => std::iter::once("Name,Sequence\n".to_string()).chain(oligos.iter().map(|o| format!("{},{}\n", o.name, o.sequence))).collect(),
    }
}

/// `text` reduced to the characters vendors accept in an oligo name
/// (letters, digits, `_`, `-` and `.`), every other run becoming `_`.
pub fn oligo_name(text: &str) -> String {
    let mut name = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '.') {
            name.push(c);
        } else if !name.ends_with('_') {
            name.push('_');
        }
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oligo(name: &str, len: usize) -> Oligo {
        Oligo { name: name.to_string(), sequence: "ACGT".repeat(len.div_ceil(4))[..len].to_string() }
    }

    #[test]
    fn idt_scales_follow_pool_size() {
        let pool = |n: usize| (0..n).map(|i| oligo(&format!("o{i}"), 120)).collect::<Vec<_>>();
        assert_eq!(check(Vendor::Idt, &pool(5)).idt_scales, vec!["50 pmol"]);
        assert_eq!(check(Vendor::Idt, &pool(200)).idt_scales, vec!["50 pmol", "10 pmol", "1 pmol"]);
        assert_eq!(check(Vendor::Idt, &pool(1_000)).idt_scales, vec!["10 pmol", "1 pmol"]);
        let one = check(Vendor::Idt, &pool(1));
        assert!(one.idt_scales.is_empty() && one.problems.iter().any(|p| p.contains("2 to 20,000")), "{:?}", one.problems);
        assert!(check(Vendor::Twist, &pool(1)).problems.is_empty());
    }

    #[test]
    fn lengths_letters_and_names_are_checked() {
        let order = [oligo("short", 30), oligo("long", 351), Oligo { name: "bad".into(), sequence: "ACGN".repeat(10) }, oligo("short", 100)];
        let idt = check(Vendor::Idt, &order);
        assert!(idt.problems.iter().any(|p| p.starts_with("short is 30 nt; IDT makes 40-350")), "{:?}", idt.problems);
        assert!(idt.problems.iter().any(|p| p.starts_with("long is 351 nt")));
        assert!(idt.problems.iter().any(|p| p.starts_with("bad contains 'N'")));
        assert!(idt.problems.iter().any(|p| p.contains("'short' is empty or used twice")));
        // Twist makes 30 nt oligos.
        assert!(!check(Vendor::Twist, &order).problems.iter().any(|p| p.starts_with("short is 30 nt")));
    }

    #[test]
    fn formats_follow_what_each_vendor_takes() {
        assert_eq!(Vendor::Twist.format_for("order.fasta"), Ok(SheetFormat::Fasta));
        assert_eq!(Vendor::Twist.format_for("order.CSV"), Ok(SheetFormat::Csv));
        assert_eq!(Vendor::Idt.format_for("order.csv"), Ok(SheetFormat::Csv));
        assert!(Vendor::Idt.format_for("order.fasta").unwrap_err().contains("Excel uploads only"));
        assert!(Vendor::Twist.format_for("order.xlsx").is_err());
        let oligos = [oligo("a_d0", 8), oligo("a_p0", 4)];
        assert_eq!(render(SheetFormat::Fasta, &oligos), ">a_d0\nACGTACGT\n>a_p0\nACGT\n");
        assert_eq!(render(SheetFormat::Csv, &oligos), "Name,Sequence\na_d0,ACGTACGT\na_p0,ACGT\n");
    }

    #[test]
    fn names_keep_only_what_vendors_accept() {
        assert_eq!(oligo_name("archive/patient records (2026).csv"), "archive_patient_records_2026_.csv");
        assert_eq!(oligo_name("p0/f1.txt"), "p0_f1.txt");
    }
}
