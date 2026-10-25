//! Charging subscriptions for their deliveries: once a week, when the
//! invoice job gets to them, and again when Stripe says a payment failed
//! and the customer's card has been updated since.

use crate::db::Db;
use crate::{Error, stripe};

/// The amount a subscription owes for a period, in öre.
pub async fn amount_due(db: &Db, subscription: &str, period: &str) -> Result<i64, Error> {
    let lines = db.invoice_lines(subscription, period).await?;
    Ok(lines.iter().map(|l| l.amount).sum())
}

/// Charges `subscription` for `period`. The invoice job calls it for every
/// subscription; the `invoice.payment_failed` webhook for those whose card
/// was declined, once the customer has a new one.
pub async fn charge_once(db: &Db, subscription: &str, period: &str) -> Result<String, Error> {
    let amount = amount_due(db, subscription, period).await?;
    let charge = stripe::charge(subscription, amount).await?;
    db.record_charge(subscription, period, &charge.id).await?;
    Ok(charge.id)
}
