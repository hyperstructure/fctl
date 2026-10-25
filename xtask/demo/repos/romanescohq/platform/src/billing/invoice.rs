//! Invoices: what a subscription owes for a week, line by line.

use chrono::{DateTime, Datelike, Days, NaiveDate, Utc};

/// A subscription's invoice for the week its box goes out in, from Monday
/// to Sunday.
#[derive(Debug, PartialEq)]
pub struct Invoice {
    pub subscription: String,
    pub issued: NaiveDate,
    pub period: Period,
    pub lines: Vec<Line>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Period {
    pub start: NaiveDate,
    pub end: NaiveDate,
}

#[derive(Debug, PartialEq)]
pub struct Line {
    pub text: String,
    /// In öre, VAT included.
    pub amount: i64,
    /// In percent.
    pub vat_rate: u8,
}

/// The week `now` falls in.
pub fn week_of(now: DateTime<Utc>) -> Period {
    let today = now.date_naive();
    let start = today - Days::new(u64::from(today.weekday().num_days_from_monday()));
    Period { start, end: start + Days::new(6) }
}

/// The invoice for a box's `items`, for the week it goes out in.
pub fn invoice_for(subscription: &str, items: &[crate::BoxItem]) -> Invoice {
    let lines = items
        .iter()
        .map(|i| Line { text: i.name.clone(), amount: i.price, vat_rate: 12 })
        .collect();
    Invoice {
        subscription: subscription.to_owned(),
        issued: Utc::now().date_naive(),
        period: week_of(Utc::now()),
        lines,
    }
}

/// The VAT an invoice's lines include, in öre: food's 12%.
pub fn vat(invoice: &Invoice) -> i64 {
    let total: i64 = invoice.lines.iter().map(|l| l.amount).sum();
    total * 12 / 112
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BoxItem;

    #[test]
    fn invoices_the_week_of_the_box() {
        let invoice = invoice_for("sub_18342", &[BoxItem::test("Kale", 3500)]);
        assert_eq!(invoice.issued, Utc::now().date_naive());
        assert_eq!(invoice.period, week_of(Utc::now()));
    }

    #[test]
    fn a_week_runs_from_monday_to_sunday() {
        let p = week_of("2026-10-21T10:00:00Z".parse().unwrap());
        assert_eq!(p.start.weekday(), chrono::Weekday::Mon);
        assert_eq!(p.end.weekday(), chrono::Weekday::Sun);
    }
}
