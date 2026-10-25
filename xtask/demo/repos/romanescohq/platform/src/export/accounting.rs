//! The week's sales for the accounting system: rows of CSV, one an
//! invoice, booked to the BAS account for food at 12%.

use crate::billing::invoice::{self, Invoice};

/// The header the accounting system's import expects.
pub const HEADER: &str = "subscription,period_end,account,amount,vat";

/// An invoice's row.
pub fn rows(invoice: &Invoice) -> Vec<String> {
    let total: i64 = invoice.lines.iter().map(|l| l.amount).sum();
    let vat = invoice::vat(invoice);
    vec![format!("{},{},3002,{},{}", invoice.subscription, invoice.period.end, ore(total), ore(vat))]
}

/// Öre as the import takes amounts: kronor with two decimals.
fn ore(amount: i64) -> String {
    format!("{}.{:02}", amount / 100, amount % 100)
}
