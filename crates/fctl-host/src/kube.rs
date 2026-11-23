//! A host in a Kubernetes pod (docs/self-hosting.md#hosts-in-kubernetes)
//! keeps its pod's deletion cost at the number of sessions it runs
//! (`controller.kubernetes.io/pod-deletion-cost`), so that a Deployment
//! that scales down, as KEDA scales one to a pool's demand, removes the
//! pods that run none first. It patches its own pod with its service
//! account's token, which needs `patch` on `pods`; refused, it stops trying
//! and says why once.

use fctl_client::HttpClient;
use http::Method;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where a pod's service account's token, namespace and cluster
/// certificate authority are.
const ACCOUNT: &str = "/var/run/secrets/kubernetes.io/serviceaccount";

/// The annotation a ReplicaSet reads when it scales down: pods with a
/// lower cost go first.
pub const DELETION_COST: &str = "controller.kubernetes.io/pod-deletion-cost";

/// The pod this host runs in.
pub struct Pod {
    /// `https://host:port` of the cluster's API server.
    server: String,
    namespace: String,
    name: String,
    token: PathBuf,
    client: HttpClient,
}

impl Pod {
    /// The pod this host runs in, when it runs in one: Kubernetes sets
    /// `KUBERNETES_SERVICE_HOST` in every pod, and mounts its service
    /// account. Its name is `POD_NAME` (the downward API's
    /// `metadata.name`), else the container's hostname (`HOSTNAME`, else
    /// `/etc/hostname`), which is the pod's name; its namespace
    /// `POD_NAMESPACE`, else its service account's.
    pub fn here() -> Option<Self> {
        Self::from(
            |k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()),
            Path::new(ACCOUNT),
            Path::new("/etc/hostname"),
        )
    }

    fn from(
        var: impl Fn(&str) -> Option<String>,
        account: &Path,
        etc_hostname: &Path,
    ) -> Option<Self> {
        let host = var("KUBERNETES_SERVICE_HOST")?;
        let port = var("KUBERNETES_SERVICE_PORT").unwrap_or_else(|| "443".into());
        let ca = std::fs::read(account.join("ca.crt")).ok()?;
        let tls = match fctl_client::tls_config_trusting(&ca) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("the cluster's certificate authority: {e}");
                return None;
            }
        };
        let namespace = var("POD_NAMESPACE").or_else(|| {
            std::fs::read_to_string(account.join("namespace"))
                .ok()
                .map(|n| n.trim().to_owned())
        })?;
        let name = var("POD_NAME").or_else(|| var("HOSTNAME")).or_else(|| {
            std::fs::read_to_string(etc_hostname)
                .ok()
                .map(|h| h.trim().to_owned())
                .filter(|h| !h.is_empty())
        })?;
        // An IPv6 address goes in brackets.
        let host = if host.contains(':') {
            format!("[{host}]")
        } else {
            host
        };
        Some(Self {
            server: format!("https://{host}:{port}"),
            namespace,
            name,
            token: account.join("token"),
            client: HttpClient::with_tls(Duration::from_secs(10), &tls),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Sets the pod's deletion cost to `sessions`. Refused (401, 403, 404)
    /// is `Err` with `true`, and says what to change: it won't work later
    /// either.
    pub async fn set_deletion_cost(&self, sessions: usize) -> Result<(), (String, bool)> {
        // Projected tokens rotate: read it each time.
        let token = std::fs::read_to_string(&self.token)
            .map_err(|e| (format!("reading the service account's token: {e}"), true))?;
        let url = format!(
            "{}/api/v1/namespaces/{}/pods/{}",
            self.server, self.namespace, self.name
        );
        let (status, _, body) = self
            .client
            .send(
                Method::PATCH,
                &url,
                Some(token.trim()),
                Some(patch(sessions).into_bytes()),
                Some("application/merge-patch+json"),
            )
            .await
            .map_err(|e| (e.to_string(), false))?;
        if status.is_success() {
            return Ok(());
        }
        let said = String::from_utf8_lossy(&body);
        let message = serde_json::from_str::<serde_json::Value>(&said)
            .ok()
            .and_then(|v| v.get("message")?.as_str().map(str::to_owned))
            .unwrap_or_else(|| said.chars().take(300).collect());
        let fix = match status.as_u16() {
            401 | 403 => "; its service account needs `patch` on pods",
            404 => "; set POD_NAME from the downward API (metadata.name)",
            _ => "",
        };
        let lasting = !fix.is_empty();
        Err((format!("{status}: {message}{fix}"), lasting))
    }
}

/// The merge patch that sets the deletion cost to `sessions`.
fn patch(sessions: usize) -> String {
    serde_json::json!({
        "metadata": { "annotations": { DELETION_COST: sessions.to_string() } }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pod_is_found_by_its_environment_and_service_account() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let etc = dir.path().join("hostname");
        let none = |_: &str| None::<String>;
        // Outside a pod: nothing.
        assert!(Pod::from(none, dir.path(), &etc).is_none());
        // A pod without its service account mounted: nothing.
        let env = |k: &str| match k {
            "KUBERNETES_SERVICE_HOST" => Some("10.96.0.1".to_owned()),
            "POD_NAME" => Some("fctl-host-7d9c-x2k4p".to_owned()),
            _ => None,
        };
        assert!(Pod::from(env, dir.path(), &etc).is_none());
        // A self-signed authority stands in for the cluster's.
        let ca = rcgen::generate_simple_self_signed(vec!["kubernetes".to_owned()])?;
        std::fs::write(dir.path().join("ca.crt"), ca.cert.pem())?;
        std::fs::write(dir.path().join("namespace"), "fctl\n")?;
        std::fs::write(dir.path().join("token"), "t")?;
        let pod = Pod::from(env, dir.path(), &etc).ok_or_else(|| anyhow::anyhow!("no pod"))?;
        assert_eq!(pod.name(), "fctl-host-7d9c-x2k4p");
        assert_eq!(pod.namespace, "fctl");
        assert_eq!(pod.server, "https://10.96.0.1:443");
        let v6 = |k: &str| match k {
            "KUBERNETES_SERVICE_HOST" => Some("fd00::1".to_owned()),
            "KUBERNETES_SERVICE_PORT" => Some("6443".to_owned()),
            "POD_NAMESPACE" => Some("runners".to_owned()),
            _ => None,
        };
        // Without POD_NAME, the container's hostname, never the runner's
        // name (FCTL_HOSTNAME); without either, no pod to patch.
        assert!(Pod::from(v6, dir.path(), &etc).is_none());
        std::fs::write(&etc, "fctl-host-5f6b-q9z2w\n")?;
        let pod = Pod::from(v6, dir.path(), &etc).ok_or_else(|| anyhow::anyhow!("no pod"))?;
        assert_eq!(pod.server, "https://[fd00::1]:6443");
        assert_eq!(pod.namespace, "runners");
        assert_eq!(pod.name(), "fctl-host-5f6b-q9z2w");
        let named = |k: &str| match k {
            "HOSTNAME" => Some("fctl-host-1a2b-c3d4e".to_owned()),
            "FCTL_HOSTNAME" => Some("build-7".to_owned()),
            k => v6(k),
        };
        let pod = Pod::from(named, dir.path(), &etc).ok_or_else(|| anyhow::anyhow!("no pod"))?;
        assert_eq!(pod.name(), "fctl-host-1a2b-c3d4e");
        Ok(())
    }

    #[test]
    fn the_patch_sets_the_annotation_as_a_string() {
        assert_eq!(
            patch(3),
            r#"{"metadata":{"annotations":{"controller.kubernetes.io/pod-deletion-cost":"3"}}}"#
        );
    }
}
