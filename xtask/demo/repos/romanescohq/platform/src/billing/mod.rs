//! Billing: invoices, charges and what Stripe says about them.

pub mod charge;
pub mod invoice;
pub mod receipt;

use crate::db::Db;

/// `invoice.paid`: the period is settled.
pub async fn paid(db: &Db, event: &stripe::Event) -> Result<(), crate::Error> {
    db.mark_paid(&event.invoice_id()?).await
}

/// `invoice.payment_failed`: the card was declined. Once the customer has
/// a new one, the period is charged again.
pub async fn payment_failed(db: &Db, event: &stripe::Event) -> Result<(), crate::Error> {
    let (subscription, period) = event.subscription_period()?;
    if db.card_updated_since(&subscription, event.created()).await? {
        charge::charge_once(db, &subscription, &period).await?;
    }
    Ok(())
}

/// `charge.refunded`: support refunded some or all of a charge.
pub async fn refunded(db: &Db, event: &stripe::Event) -> Result<(), crate::Error> {
    db.record_refund(&event.charge_id()?, event.amount_refunded()).await
}
