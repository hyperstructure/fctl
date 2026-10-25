//! Stripe's webhooks: invoices, charges and subscriptions changing.

use crate::{db::Db, jobs};

pub async fn stripe(db: Db, event: stripe::Event) -> Result<(), crate::Error> {
    if let Err(e) = process(&db, &event).await {
        tracing::error!(event = %event.id, "processing a Stripe event failed: {e:#}");
    }
    Ok(())
}

async fn process(db: &Db, event: &stripe::Event) -> Result<(), crate::Error> {
    match event.type_.as_str() {
        "invoice.paid" => crate::billing::paid(db, event).await,
        "invoice.payment_failed" => crate::billing::payment_failed(db, event).await,
        "charge.refunded" => crate::billing::refunded(db, event).await,
        _ => Ok(()),
    }
}
