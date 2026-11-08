//! Downloads that fail in a way that may pass, tried again: in CI, Docker
//! Hub's CDN reset a connection halfway through a layer on 2026-11-07, and
//! GitHub's release downloads answered 504 for half an hour the day after.
//! A VM can't start without what the driver downloads (Firecracker, the
//! kernel, an image's manifests and layers), so one such failure would
//! fail a session's start.

use super::Log;
use std::future::Future;
use std::path::Path;
use std::time::Duration;

/// How many times a download is tried, and how long it waits before the
/// second try: three times longer before each try after that.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retries {
    pub tries: u32,
    pub first_wait: Duration,
}

impl Retries {
    /// Five tries, 2, 6, 18 and 54 s apart: a download still failing a
    /// minute and a half after its first try fails.
    pub const DOWNLOADS: Self = Self {
        tries: 5,
        first_wait: Duration::from_secs(2),
    };

    /// The wait after the `failed`th try in a row failed.
    pub fn wait(&self, failed: u32) -> Duration {
        self.first_wait
            .saturating_mul(3u32.saturating_pow(failed.saturating_sub(1)))
    }

    /// What `attempt` returns once it is something other than a failure
    /// that may pass, which `may_pass` says why it is, or once it has been
    /// tried [`tries`](Self::tries) times. `log` says when it tries again,
    /// and why: "`what` failed (why): trying again in 2 s".
    pub async fn run<T, F, Fut>(
        &self,
        what: &str,
        log: Log<'_>,
        mut attempt: F,
        may_pass: impl Fn(&T) -> Option<String>,
    ) -> T
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = T>,
    {
        let mut failed = 0;
        loop {
            let out = attempt().await;
            failed += 1;
            match may_pass(&out) {
                Some(why) if failed < self.tries => {
                    let wait = self.wait(failed);
                    log(format!(
                        "{what} failed ({why}): trying again in {} s",
                        wait.as_secs_f64().ceil()
                    ));
                    tokio::time::sleep(wait).await;
                }
                _ => return out,
            }
        }
    }
}

/// Why `e` may pass on another try, when it may: a network error, a
/// timeout, a 5xx or a 429.
pub fn may_pass(e: &fctl_client::Error) -> Option<String> {
    e.is_transient().then(|| brief(&e.to_string()))
}

/// The first line of `message`, at most 200 characters: an error page's
/// HTML, say, cut short.
pub fn brief(message: &str) -> String {
    let line = message.lines().next().unwrap_or_default().trim();
    match line.char_indices().nth(200) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line.to_owned(),
    }
}

/// Downloads into the file `to` what each of `urls` serves, the same file:
/// from the first, then from the next one each time it fails, from the
/// start each time, and from the first again after the last, until
/// [`Retries::tries`] tries have failed. A place whose failure won't pass
/// on another try (a refusal, say, as GitHub's API refuses those who made
/// too many requests) is tried no more, and once none is left, it fails.
/// What it says then is what its last try said. At most `max_bytes`, each
/// try within `limit`. `what` names it in what `log` says.
pub async fn download(
    urls: &[&str],
    to: &Path,
    max_bytes: u64,
    limit: Duration,
    retries: Retries,
    what: &str,
    log: Log<'_>,
) -> fctl_client::http::Result<u64> {
    let http = fctl_client::HttpClient::new(limit);
    // GitHub's API sends a release asset's file, not JSON about it, when it
    // is asked for that.
    let headers = [("accept", "application/octet-stream")];
    let mut places: Vec<&str> = urls.to_vec();
    let mut next = 0;
    let mut last = None;
    for tried in 1..=retries.tries {
        let Some(&url) = places.get(next) else {
            break;
        };
        let e = match http
            .download_to(url, None, &headers, to, max_bytes, limit)
            .await
        {
            Ok(size) => return Ok(size),
            Err(e) => e,
        };
        let why = match may_pass(&e) {
            Some(why) => {
                next += 1;
                why
            }
            None => {
                places.remove(next);
                brief(&e.to_string())
            }
        };
        if next >= places.len() {
            next = 0;
        }
        last = Some(e);
        if places.is_empty() || tried == retries.tries {
            break;
        }
        let wait = retries.wait(tried);
        log(format!(
            "{what} failed ({why}): trying again in {} s",
            wait.as_secs_f64().ceil()
        ));
        tokio::time::sleep(wait).await;
    }
    Err(last.unwrap_or_else(|| fctl_client::Error::Other("nowhere to download from".into())))
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    /// Retries that don't keep a test waiting.
    pub const QUICK: Retries = Retries {
        tries: 3,
        first_wait: Duration::from_millis(5),
    };

    /// A server on this machine that answers its `n`th request (from 0)
    /// with `answer(n)`, a status line and a body; and how many came.
    pub async fn server(
        answer: impl Fn(usize) -> (&'static str, Vec<u8>) + Send + Sync + 'static,
    ) -> anyhow::Result<(String, Arc<AtomicUsize>)> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let count = Arc::new(AtomicUsize::new(0));
        let (counted, answer) = (count.clone(), Arc::new(answer));
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let n = counted.fetch_add(1, Ordering::SeqCst);
                let answer = answer.clone();
                tokio::spawn(async move {
                    let (r, mut w) = stream.into_split();
                    let mut r = BufReader::new(r);
                    loop {
                        let mut line = String::new();
                        if r.read_line(&mut line).await.unwrap_or(0) == 0 || line == "\r\n" {
                            break;
                        }
                    }
                    let (status, body) = answer(n);
                    let head = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = w.write_all(head.as_bytes()).await;
                    let _ = w.write_all(&body).await;
                });
            }
        });
        Ok((url, count))
    }

    #[test]
    fn waits_three_times_longer_each_time() {
        let waits: Vec<u64> = (1..5)
            .map(|n| Retries::DOWNLOADS.wait(n).as_secs())
            .collect();
        assert_eq!(waits, [2, 6, 18, 54]);
        assert_eq!(Retries::DOWNLOADS.tries, 5);
    }

    #[test]
    fn cuts_error_pages_short() {
        assert_eq!(
            brief("HTTP 504: <html><body><h1>504 Gateway Time-out</h1>\nThe server didn't respond"),
            "HTTP 504: <html><body><h1>504 Gateway Time-out</h1>"
        );
        let long = "x".repeat(300);
        assert_eq!(brief(&long).chars().count(), 201);
    }

    /// GitHub's release downloads answered 504 for a while: a download
    /// tries again, and what came of the try that worked is in the file.
    #[tokio::test]
    async fn downloads_again_what_failed_in_a_way_that_may_pass() -> anyhow::Result<()> {
        let (url, count) = server(|n| match n {
            0 => ("504 Gateway Time-out", b"<html>504</html>".to_vec()),
            1 => ("503 Service Unavailable", Vec::new()),
            _ => ("200 OK", b"the release".to_vec()),
        })
        .await?;
        let dir = tempfile::tempdir()?;
        let to = dir.path().join("archive");
        let said = std::sync::Mutex::new(Vec::new());
        let log = |line: String| said.lock().unwrap_or_else(|e| e.into_inner()).push(line);
        let url = format!("{url}/firecracker.tgz");
        let got = download(
            &[&url],
            &to,
            1024,
            Duration::from_secs(10),
            QUICK,
            "Downloading Firecracker",
            &log,
        )
        .await?;
        assert_eq!(got, 11);
        assert_eq!(std::fs::read(&to)?, b"the release");
        assert_eq!(count.load(Ordering::SeqCst), 3);
        let said = said.into_inner().unwrap_or_default();
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(
            said[0].starts_with(
                "Downloading Firecracker failed (HTTP 504: <html>504</html>): trying again in"
            ),
            "{said:?}"
        );
        Ok(())
    }

    /// GitHub's release downloads answered 504 for half an hour while its
    /// API served the same file: the next try is from the next place.
    #[tokio::test]
    async fn downloads_from_the_next_place_while_one_fails() -> anyhow::Result<()> {
        let (failing, failed) = server(|_| ("504 Gateway Time-out", Vec::new())).await?;
        let (working, worked) = server(|_| ("200 OK", b"the release".to_vec())).await?;
        let dir = tempfile::tempdir()?;
        let to = dir.path().join("archive");
        let urls = [failing.as_str(), working.as_str()];
        download(
            &urls,
            &to,
            1024,
            Duration::from_secs(10),
            QUICK,
            "x",
            &|_| {},
        )
        .await?;
        assert_eq!(std::fs::read(&to)?, b"the release");
        assert_eq!(
            (failed.load(Ordering::SeqCst), worked.load(Ordering::SeqCst)),
            (1, 1)
        );
        // From the first again after the last.
        let (other, again) = server(|_| ("503 Service Unavailable", Vec::new())).await?;
        let urls = [other.as_str(), failing.as_str()];
        let e = download(
            &urls,
            &to,
            1024,
            Duration::from_secs(10),
            QUICK,
            "x",
            &|_| {},
        )
        .await;
        assert!(
            matches!(e, Err(fctl_client::Error::Status { status: 503, .. })),
            "{e:?}"
        );
        assert_eq!(
            (again.load(Ordering::SeqCst), failed.load(Ordering::SeqCst)),
            (2, 2)
        );
        Ok(())
    }

    /// A place that refuses (GitHub's API, which lets those without a token
    /// make 60 requests an hour) is tried no more, and the others are
    /// tried again: what it says in the end is what its last try said.
    #[tokio::test]
    async fn a_place_that_refuses_is_tried_no_more() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let to = dir.path().join("archive");
        let get = async |urls: &[&str]| {
            download(
                urls,
                &to,
                1024,
                Duration::from_secs(10),
                QUICK,
                "x",
                &|_| {},
            )
            .await
        };
        let status = |e: fctl_client::http::Result<u64>| match e {
            Err(fctl_client::Error::Status { status, .. }) => Some(status),
            _ => None,
        };
        let count = |n: &Arc<AtomicUsize>| n.load(Ordering::SeqCst);
        let (failing, failed) = server(|_| ("504 Gateway Time-out", Vec::new())).await?;
        let (refusing, refused) = server(|_| ("403 Forbidden", b"rate limit".to_vec())).await?;
        let (missing, gone) = server(|_| ("404 Not Found", Vec::new())).await?;
        let (working, worked) = server(|_| ("200 OK", b"the release".to_vec())).await?;
        // 504, 403, then the first again: its 504.
        assert_eq!(status(get(&[&failing, &refusing]).await), Some(504));
        assert_eq!((count(&failed), count(&refused)), (2, 1));
        // The first refusing, the next has it.
        get(&[&refusing, &working]).await?;
        assert_eq!(std::fs::read(&to)?, b"the release");
        assert_eq!((count(&refused), count(&worked)), (2, 1));
        // All refusing: it stops, with the last refusal.
        assert_eq!(status(get(&[&missing, &refusing]).await), Some(403));
        assert_eq!((count(&gone), count(&refused)), (1, 3));
        // What the place still tried said, not the one that refused first.
        assert_eq!(status(get(&[&missing, &failing]).await), Some(504));
        assert_eq!((count(&gone), count(&failed)), (2, 4));
        Ok(())
    }

    /// What won't pass on another try fails at once, and what keeps
    /// failing fails after the last try, with what it said then.
    #[tokio::test]
    async fn gives_up_on_what_wont_pass() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let to = dir.path().join("archive");
        let (url, count) = server(|_| ("404 Not Found", b"no such release".to_vec())).await?;
        let e = download(
            &[&url],
            &to,
            1024,
            Duration::from_secs(10),
            QUICK,
            "x",
            &|_| {},
        )
        .await;
        assert!(
            matches!(e, Err(fctl_client::Error::Status { status: 404, .. })),
            "{e:?}"
        );
        assert_eq!(count.load(Ordering::SeqCst), 1);
        let (url, count) = server(|_| ("502 Bad Gateway", Vec::new())).await?;
        let e = download(
            &[&url],
            &to,
            1024,
            Duration::from_secs(10),
            QUICK,
            "x",
            &|_| {},
        )
        .await;
        assert!(
            matches!(e, Err(fctl_client::Error::Status { status: 502, .. })),
            "{e:?}"
        );
        assert_eq!(count.load(Ordering::SeqCst), 3);
        // Nobody there: a network error, tried again.
        let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/", closed.local_addr()?);
        drop(closed);
        let tries = AtomicUsize::new(0);
        let log = |_: String| {
            tries.fetch_add(1, Ordering::SeqCst);
        };
        let e = download(
            &[&url],
            &to,
            1024,
            Duration::from_secs(10),
            QUICK,
            "x",
            &log,
        )
        .await;
        assert!(
            matches!(e, Err(fctl_client::Error::Network { .. })),
            "{e:?}"
        );
        assert_eq!(tries.load(Ordering::SeqCst), 2);
        Ok(())
    }
}
