//! Charging subscriptions for their deliveries.

pub async fn charge_once(db: &crate::db::Db, subscription: &str, period: &str) -> Result<(), crate::Error> {
    let charge = crate::stripe::charge(subscription, period).await?;
    db.record_charge(subscription, period, &charge.id).await
}
