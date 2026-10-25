//! The invoice job: every morning at 06:00, an invoice and a charge for
//! each subscription whose box goes out that day.

use crate::billing::{charge, invoice};
use crate::db::Db;

/// Invoices and charges the day's boxes. One runs at a time: the job holds
/// an advisory lock while it works, and a second waits for it.
pub async fn run(db: &Db) -> Result<(), crate::Error> {
    sqlx::query("SELECT pg_advisory_lock(4242)").execute(db.pool()).await?;
    for sub in db.subscriptions_delivering_today().await? {
        let items = db.box_items(&sub.id).await?;
        let invoice = invoice::invoice_for(&sub.id, &items);
        db.save_invoice(&invoice).await?;
        charge::charge_once(db, &sub.id, &invoice.period.key()).await?;
    }
    sqlx::query("SELECT pg_advisory_unlock(4242)").execute(db.pool()).await?;
    Ok(())
}
