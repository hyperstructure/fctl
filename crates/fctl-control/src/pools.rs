//! Runner pools' demand (docs/architecture.md#runner-pools), as FleetDO
//! counts it from what their runners do and the sessions that wait, for
//! the enrollment-token holder, the workspace's admins and service
//! accounts, and the Prometheus endpoint.

use crate::ctx::Ctx;
use crate::error::AppResult;
use crate::internal;
use fctl_proto::api::WorkspaceDemand;

/// What a workspace's pools need now: every one of them, or those of the
/// enrollment tokens `pools` names.
pub async fn workspace_demand(
    ctx: &Ctx,
    workspace_id: &str,
    pools: Option<&[String]>,
) -> AppResult<WorkspaceDemand> {
    demand_of(&ctx.env, workspace_id, pools).await
}

/// [`workspace_demand`], with the Worker's environment alone.
pub async fn demand_of(
    env: &worker::Env,
    workspace_id: &str,
    pools: Option<&[String]>,
) -> AppResult<WorkspaceDemand> {
    let stub = internal::fleet_stub(env)?;
    internal::post(
        &stub,
        "/demand",
        &internal::DemandOf {
            workspace_id: workspace_id.to_owned(),
            pools: pools.map(<[String]>::to_vec),
        },
    )
    .await
}
