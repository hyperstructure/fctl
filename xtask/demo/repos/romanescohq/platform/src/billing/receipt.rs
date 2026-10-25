//! The receipt in each delivery's email: the box's items, the total, and
//! the VAT it includes.

use super::invoice::{self, Invoice};
use crate::money::kronor;

/// The receipt's lines, as the email's plain-text part shows them.
pub fn lines(invoice: &Invoice) -> Vec<String> {
    let mut out: Vec<String> = invoice
        .lines
        .iter()
        .map(|l| format!("{:<32}{:>10}", l.text, kronor(l.amount)))
        .collect();
    let total: i64 = invoice.lines.iter().map(|l| l.amount).sum();
    out.push(format!("{:<32}{:>10}", "Totalt", kronor(total)));
    out.push(format!("{:<32}{:>10}", "Varav moms 12 %", kronor(invoice::vat(invoice))));
    out
}
