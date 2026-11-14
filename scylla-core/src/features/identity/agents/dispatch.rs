//! The dispatch of a job to an agent: its nodes with the values of their secrets, and the checkout
//! of its code with a short-lived token.

use scylla_domain::projects::pipelines::Step;
use scylla_proto::agents::v1 as pb;
use scylla_proto::agents::v1::open_stream_response::Message as Down;
use scylla_proto::pipelines::v1 as pipelines_pb;

use crate::api::parse::ToProto;
use crate::features::integrations::checkout::Checkout;
use crate::features::projects::jobs::runs::Assignment;
use crate::features::projects::pipelines;

/// Makes the dispatch of a job, with the values of its secrets and its checkout.
pub(super) fn dispatch(assignment: Assignment, checkout: Option<Checkout>) -> Down {
    let Assignment { job, env } = assignment;
    let nodes = job
        .nodes
        .into_iter()
        .zip(env)
        .map(|(node, env)| pb::DispatchNode {
            id: node.id.as_str().to_owned(),
            needs: node
                .needs
                .iter()
                .map(|need| need.as_str().to_owned())
                .collect(),
            step: Some(match node.step {
                Step::Exec(argv) => pb::dispatch_node::Step::Exec(pipelines_pb::Exec { argv }),
                Step::Script { shell, source } => {
                    pb::dispatch_node::Step::Script(pipelines::script(shell, source))
                }
            }),
            env: env
                .into_iter()
                .map(|var| pb::DispatchEnv {
                    name: var.name,
                    value: var.value,
                    masked: var.masked,
                })
                .collect(),
            timeout_seconds: node.timeout_seconds,
            retries: node.retries,
            working_dir: node.working_dir.unwrap_or_default(),
            container: pipelines::container(node.runtime),
        })
        .collect();
    Down::Dispatch(pb::Dispatch {
        job_id: Some(job.id.to_proto()),
        pipeline_id: Some(job.pipeline_id.to_proto()),
        project_id: Some(job.project_id.to_proto()),
        nodes,
        checkout: checkout.map(checkout_of),
    })
}

/// Makes the checkout of a dispatch.
fn checkout_of(checkout: Checkout) -> pb::DispatchCheckout {
    pb::DispatchCheckout {
        clone_url: checkout.clone_url,
        sha: checkout.sha,
        fetch_ref: checkout.fetch_ref,
        auth_header: checkout.auth_header.expose().to_owned(),
        depth: checkout.settings.depth,
        submodules: checkout.settings.submodules,
        lfs: checkout.settings.lfs,
    }
}
