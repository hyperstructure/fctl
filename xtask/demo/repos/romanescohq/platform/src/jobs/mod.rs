//! Work that can wait: done by the job runner, retried with backoff.

pub enum Job {
    PlanRoutes { day: String },
    SendReminder { subscription: String },
}

pub async fn enqueue(db: &crate::db::Db, job: Job) -> Result<(), crate::Error> {
    db.jobs().push(job).await
}
