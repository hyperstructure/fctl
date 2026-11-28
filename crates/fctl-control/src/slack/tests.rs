//! What the Slack app keeps in D1, against the stand-in for D1
//! (`crate::testing`): people's accounts, installations and their
//! revocation, which projects a request may start a session in, the
//! thread a session answers in, what agents reach, their limits, and what
//! expires.

use super::inbound::{self, Ask, AskHow};
use crate::crypto::Keys;
use crate::db::{n, q, s};
use crate::error::AppResult;
use crate::model::{UserRow, user_columns};
use fctl_domain::slack::access::CallKind;
use fctl_proto::slack::{SlackPlace, SlackSettings, SlackVia};
use wasm_bindgen_test::wasm_bindgen_test;
use worker::D1Database;

const NOW: i64 = 1_800_000_000_000;

fn keys() -> Keys {
    Keys::derive(&[7u8; 32])
}

fn cfg() -> crate::config::SlackConfig {
    crate::config::SlackConfig {
        client_id: "1.2".into(),
        client_secret: "secret".into(),
        signing_secret: Some("signing".into()),
        app_token: None,
        url: "http://127.0.0.1:9".into(),
    }
}

fn bot() -> super::Slack {
    super::Slack::with_token(&cfg(), "xoxb-1", "T1", "UBOT")
}

/// Acme's workspace, connected to Slack's T1, with its projects web and
/// docs, an archived one, and another workspace's ops; Ada an admin of
/// Acme, Grace a member who may start sessions only in web.
async fn world(db: &D1Database) -> AppResult<()> {
    for sql in [
        "INSERT INTO users (id, email, name, role, created_at, updated_at) VALUES \
         ('usr_ada', 'ada@acme.dev', 'Ada Lovelace', 'member', 0, 0), ('usr_grace', 'grace@acme.dev', 'Grace Hopper', 'member', 0, 0), \
         ('usr_bob', 'bob@else.dev', 'Bob', 'member', 0, 0)",
        "INSERT INTO user_emails (email, user_id, created_at) VALUES ('ada@acme.dev', 'usr_ada', 0), ('grace@acme.dev', 'usr_grace', 0), ('bob@else.dev', 'usr_bob', 0)",
        "INSERT INTO workspaces (id, slug, name, base_role, created_at, updated_at) VALUES \
         ('wsp_acme', 'acme', 'Acme', 'none', 0, 0), ('wsp_else', 'else', 'Else', 'editor', 0, 0)",
        "INSERT INTO workspace_members (workspace_id, user_id, email, role, created_at) VALUES \
         ('wsp_acme', 'usr_ada', 'ada@acme.dev', 'admin', 0), ('wsp_acme', 'usr_grace', 'grace@acme.dev', 'member', 0), \
         ('wsp_else', 'usr_bob', 'bob@else.dev', 'member', 0)",
        "INSERT INTO projects (id, slug, name, created_by, created_at, updated_at, workspace_id) VALUES \
         ('prj_web', 'web', 'Web', 'usr_ada', 0, 0, 'wsp_acme'), ('prj_docs', 'docs', 'Docs', 'usr_ada', 0, 0, 'wsp_acme'), \
         ('prj_ops', 'ops', 'Ops', 'usr_bob', 0, 0, 'wsp_else')",
        "INSERT INTO projects (id, slug, name, created_by, created_at, updated_at, workspace_id, archived_at) VALUES \
         ('prj_old', 'old', 'Old', 'usr_ada', 0, 0, 'wsp_acme', 5)",
        "INSERT INTO project_members (project_id, user_id, role, created_at) VALUES \
         ('prj_web', 'usr_grace', 'editor', 0), ('prj_docs', 'usr_grace', 'viewer', 0)",
        "INSERT INTO sessions (id, project_id, number, title, prompt, status, created_by, claude_session_id, created_at, updated_at, slack) VALUES \
         ('ses_1', 'prj_web', 1, 'Fix it', 'x', 'idle', 'usr_ada', 'c1', 1, 0, '{\"team\":\"T1\",\"channel\":\"C1\",\"place\":\"channel\",\"thread_ts\":\"1.1\",\"url\":\"u\"}')",
    ] {
        q(db, sql, &[])?.run().await?;
    }
    let installed = fctl_domain::slack::Installed {
        team: "T1".into(),
        team_name: "Acme".into(),
        bot_token: "xoxb-1".into(),
        bot_user: "UBOT".into(),
        app_id: Some("A1".into()),
        scopes: vec!["chat:write".into()],
        installer: Some("U_ADA".into()),
        enterprise: false,
    };
    super::store(
        db,
        &keys(),
        &super::Team {
            id: "T1".into(),
            name: "Acme".into(),
            domain: Some("acme".into()),
            icon_url: None,
        },
        &installed,
        Some("B1"),
        "usr_ada",
        NOW,
    )
    .await?;
    q(
        db,
        "INSERT INTO workspace_slack (workspace_id, team_id, connected_by, settings, created_at) VALUES ('wsp_acme', 'T1', 'usr_ada', ?, 0)",
        &[s(&serde_json::to_string(&SlackSettings::default())?)],
    )?
    .run()
    .await?;
    Ok(())
}

async fn user(db: &D1Database, id: &str) -> AppResult<UserRow> {
    q(
        db,
        &format!("SELECT {} FROM users u WHERE u.id = ?", user_columns("u")),
        &[s(id)],
    )?
    .first()
    .await?
    .ok_or_else(missing)
}

fn missing() -> crate::error::AppError {
    crate::error::internal("missing from the test's database")
}

fn who(id: &str, name: &str) -> fctl_domain::slack::User {
    fctl_domain::slack::User {
        id: id.into(),
        name: name.into(),
        ..Default::default()
    }
}

fn ask(channel: &str, place: SlackPlace) -> Ask {
    Ask {
        team: "T1".into(),
        channel: channel.into(),
        channel_name: None,
        place,
        thread_ts: "9.9".into(),
        message_ts: "9.9".into(),
        user: "U_GRACE".into(),
        text: "<@UBOT> fix it".into(),
        how: AskHow::Mentioned,
        quoted: None,
        action_token: None,
        updates: true,
        project: None,
    }
}

#[wasm_bindgen_test]
async fn a_slack_account_is_one_persons() -> AppResult<()> {
    let db = crate::testing::database();
    world(&db).await?;
    super::keep_account(
        &db,
        "usr_ada",
        "T1",
        "Acme",
        Some("acme"),
        &who("U_ADA", "Ada"),
        SlackVia::Linked,
        NOW,
    )
    .await?;
    assert_eq!(
        super::person(&db, "T1", "U_ADA")
            .await?
            .ok_or_else(missing)?
            .id,
        "usr_ada"
    );
    assert!(super::person(&db, "T2", "U_ADA").await?.is_none());
    // Whoever proves it later has it.
    super::keep_account(
        &db,
        "usr_grace",
        "T1",
        "Acme",
        None,
        &who("U_ADA", "Ada"),
        SlackVia::Linked,
        NOW,
    )
    .await?;
    assert_eq!(
        super::person(&db, "T1", "U_ADA")
            .await?
            .ok_or_else(missing)?
            .id,
        "usr_grace"
    );
    // One account per person and Slack workspace: linking another moves it.
    super::keep_account(
        &db,
        "usr_grace",
        "T1",
        "Acme",
        None,
        &who("U_GRACE", "Grace"),
        SlackVia::Linked,
        NOW,
    )
    .await?;
    assert!(super::person(&db, "T1", "U_ADA").await?.is_none());
    // A disabled account is nobody.
    q(
        &db,
        "UPDATE users SET status = 'disabled' WHERE id = 'usr_grace'",
        &[],
    )?
    .run()
    .await?;
    assert!(super::person(&db, "T1", "U_GRACE").await?.is_none());
    Ok(())
}

#[wasm_bindgen_test]
async fn revoking_the_app_ends_every_connection() -> AppResult<()> {
    let db = crate::testing::database();
    world(&db).await?;
    let row = super::installation(&db, "T1").await?.ok_or_else(missing)?;
    assert!(row.active());
    assert_eq!(super::connected(&db, "T1").await?.len(), 1);
    super::mark_revoked(&db, "T1", NOW + 1).await?;
    assert!(
        !super::installation(&db, "T1")
            .await?
            .ok_or_else(missing)?
            .active()
    );
    assert!(super::connected(&db, "T1").await?.is_empty());
    // Installing again makes it active; the connection takes an admin's
    // connect again (`api::slack::install`).
    let installed = fctl_domain::slack::Installed {
        team: "T1".into(),
        team_name: "Acme".into(),
        bot_token: "xoxb-2".into(),
        bot_user: "UBOT".into(),
        app_id: None,
        scopes: Vec::new(),
        installer: None,
        enterprise: false,
    };
    super::store(
        &db,
        &keys(),
        &super::Team {
            id: "T1".into(),
            name: "Acme".into(),
            domain: None,
            icon_url: None,
        },
        &installed,
        None,
        "usr_ada",
        NOW + 2,
    )
    .await?;
    assert!(
        super::installation(&db, "T1")
            .await?
            .ok_or_else(missing)?
            .active()
    );
    assert!(super::connected(&db, "T1").await?.is_empty());
    Ok(())
}

#[wasm_bindgen_test]
async fn requests_start_in_the_projects_their_person_may_use() -> AppResult<()> {
    let db = crate::testing::database();
    world(&db).await?;
    let slack = bot();
    let grace = user(&db, "usr_grace").await?;
    let ada = user(&db, "usr_ada").await?;
    let ids = |c: Vec<inbound::CandidateView>| c.into_iter().map(|c| c.id).collect::<Vec<_>>();
    // Grace edits web alone; docs she only views; old is archived; ops is
    // another workspace's.
    let c = inbound::candidate_views(&db, &slack, &grace, &ask("C1", SlackPlace::Channel)).await?;
    assert_eq!(ids(c), vec!["prj_web"]);
    // Ada administers Acme: both its projects, which she may remember for
    // a channel.
    let c = inbound::candidate_views(&db, &slack, &ada, &ask("C1", SlackPlace::Channel)).await?;
    assert_eq!(ids(c.clone()), vec!["prj_docs", "prj_web"]);
    assert!(c.iter().all(|c| c.admin));
    // A channel mapped to docs offers docs alone.
    q(
        &db,
        "INSERT INTO slack_channels (workspace_id, team_id, channel_id, name, project_id, readable, created_at, updated_at) \
         VALUES ('wsp_acme', 'T1', 'C1', 'web-team', 'prj_docs', 0, 0, 0)",
        &[],
    )?
    .run()
    .await?;
    let c = inbound::candidate_views(&db, &slack, &ada, &ask("C1", SlackPlace::Channel)).await?;
    assert_eq!(ids(c), vec!["prj_docs"]);
    // ...where Grace may start nothing.
    let c = inbound::candidate_views(&db, &slack, &grace, &ask("C1", SlackPlace::Channel)).await?;
    assert!(c.is_empty());
    // Direct messages aren't mapped.
    let c = inbound::candidate_views(&db, &slack, &grace, &ask("D1", SlackPlace::Dm)).await?;
    assert_eq!(ids(c), vec!["prj_web"]);
    // A workspace that keeps direct messages from starting sessions offers
    // none there.
    let off = SlackSettings {
        direct_messages: false,
        ..SlackSettings::default()
    };
    q(
        &db,
        "UPDATE workspace_slack SET settings = ? WHERE workspace_id = 'wsp_acme'",
        &[s(&serde_json::to_string(&off)?)],
    )?
    .run()
    .await?;
    let c = inbound::candidate_views(&db, &slack, &grace, &ask("D1", SlackPlace::Dm)).await?;
    assert!(c.is_empty());
    Ok(())
}

#[wasm_bindgen_test]
async fn a_session_answers_in_its_thread_while_its_workspace_works_there() -> AppResult<()> {
    let db = crate::testing::database();
    world(&db).await?;
    q(
        &db,
        "INSERT INTO slack_threads (id, team_id, channel_id, thread_ts, place, session_id, slack_user_id, state, updates, created_at, updated_at) \
         VALUES ('T1:C1:1.1', 'T1', 'C1', '1.1', 'channel', 'ses_1', 'U_ADA', 'started', 1, 0, 0), \
                ('T1:D1:2.2', 'T1', 'D1', '2.2', 'dm', 'ses_1', 'U_ADA', 'started', 0, 0, 5)",
        &[],
    )?
    .run()
    .await?;
    // A notification's thread (no updates) isn't where it answers.
    let t = inbound::thread_of(&db, "ses_1")
        .await?
        .ok_or_else(missing)?;
    assert_eq!(t.id, "T1:C1:1.1");
    super::mark_revoked(&db, "T1", NOW).await?;
    assert!(inbound::thread_of(&db, "ses_1").await?.is_none());
    Ok(())
}

#[wasm_bindgen_test]
async fn agents_read_the_readable_channels_where_their_project_reads_them() -> AppResult<()> {
    let db = crate::testing::database();
    world(&db).await?;
    q(
        &db,
        "INSERT INTO slack_channels (workspace_id, team_id, channel_id, name, private, project_id, readable, created_at, updated_at) VALUES \
         ('wsp_acme', 'T1', 'C2', 'incidents', 0, NULL, 1, 0, 0), ('wsp_acme', 'T1', 'C3', 'deploys', 0, NULL, 1, 0, 0), \
         ('wsp_acme', 'T1', 'C4', 'web-team', 0, 'prj_web', 0, 0, 0), ('wsp_acme', 'T1', 'G1', 'secret', 1, NULL, 1, 0, 0)",
        &[],
    )?
    .run()
    .await?;
    let r = super::calls::reached(&db, "ses_1", "wsp_acme", true, None).await?;
    let names: Vec<&str> = r.readable.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["deploys", "incidents"]);
    assert_eq!(r.team.as_deref(), Some("T1"));
    assert!(r.thread.is_none());
    // A project that reads no channels reaches none.
    let r = super::calls::reached(&db, "ses_1", "wsp_acme", false, None).await?;
    assert!(r.readable.is_empty());
    // A workspace that works in no Slack workspace reaches nothing.
    let r = super::calls::reached(&db, "ses_9", "wsp_else", true, None).await?;
    assert!(r.team.is_none());
    Ok(())
}

#[wasm_bindgen_test]
async fn agents_read_and_post_so_many_times_an_hour() -> AppResult<()> {
    let db = crate::testing::database();
    world(&db).await?;
    for i in 0..fctl_domain::slack::access::limit_of(CallKind::Post) {
        assert!(super::calls::take_place(&db, "ses_1", CallKind::Post, NOW + i64::from(i)).await?);
    }
    assert!(!super::calls::take_place(&db, "ses_1", CallKind::Post, NOW + 100).await?);
    // Reads count apart, and an hour later there's room again.
    assert!(super::calls::take_place(&db, "ses_1", CallKind::Read, NOW + 100).await?);
    assert!(super::calls::take_place(&db, "ses_1", CallKind::Post, NOW + 3_600_100).await?);
    Ok(())
}

#[wasm_bindgen_test]
async fn requests_wait_an_hour() -> AppResult<()> {
    let db = crate::testing::database();
    world(&db).await?;
    q(
        &db,
        "INSERT INTO slack_threads (id, team_id, channel_id, thread_ts, place, slack_user_id, state, waiting_for, payload, expires_at, created_at, updated_at) \
         VALUES ('T1:C1:3.3', 'T1', 'C1', '3.3', 'channel', 'U_GRACE', 'waiting', 'link', '{}', ?1, ?2, ?2)",
        &[n(NOW + 10), n(NOW - 3_600_000)],
    )?
    .run()
    .await?;
    inbound::expire(&db, NOW).await?;
    assert_eq!(
        inbound::thread(&db, "T1:C1:3.3")
            .await?
            .ok_or_else(missing)?
            .state,
        "waiting"
    );
    inbound::expire(&db, NOW + 11).await?;
    let t = inbound::thread(&db, "T1:C1:3.3")
        .await?
        .ok_or_else(missing)?;
    assert_eq!(t.state, "expired");
    assert!(t.payload.is_none());
    // Forgotten a month later.
    inbound::expire(&db, NOW + 31 * 24 * 3_600_000).await?;
    assert!(inbound::thread(&db, "T1:C1:3.3").await?.is_none());
    Ok(())
}

#[wasm_bindgen_test]
async fn fctl_notes_the_newest_messages_it_passed_on() -> AppResult<()> {
    let db = crate::testing::database();
    world(&db).await?;
    q(
        &db,
        "INSERT INTO slack_threads (id, team_id, channel_id, thread_ts, place, session_id, state, created_at, updated_at) \
         VALUES ('T1:C1:1.1', 'T1', 'C1', '1.1', 'channel', 'ses_1', 'started', 0, 0)",
        &[],
    )?
    .run()
    .await?;
    for i in 0..205 {
        let t = inbound::thread(&db, "T1:C1:1.1")
            .await?
            .ok_or_else(missing)?;
        inbound::delivered(&db, &t, &format!("{i}.0"), NOW).await?;
    }
    let t = inbound::thread(&db, "T1:C1:1.1")
        .await?
        .ok_or_else(missing)?;
    let d = t.delivered();
    assert_eq!(d.len(), 200);
    assert_eq!(d.first().map(String::as_str), Some("5.0"));
    assert_eq!(d.last().map(String::as_str), Some("204.0"));
    Ok(())
}

#[wasm_bindgen_test]
async fn an_address_match_counts_while_it_is_allowed() -> AppResult<()> {
    let db = crate::testing::database();
    world(&db).await?;
    let slack = bot();
    let grace = who("U_GRACE", "Grace");
    super::keep_account(
        &db,
        "usr_grace",
        "T1",
        "Acme",
        None,
        &grace,
        SlackVia::Email,
        NOW,
    )
    .await?;
    // The instance allows it, and Acme turned it on: Grace is known.
    let mut settings = crate::instance::settings(&db).await?;
    settings.slack_match_email = true;
    crate::instance::save(&db, &settings).await?;
    let on = SlackSettings {
        match_email: true,
        ..SlackSettings::default()
    };
    q(
        &db,
        "UPDATE workspace_slack SET settings = ? WHERE workspace_id = 'wsp_acme'",
        &[s(&serde_json::to_string(&on)?)],
    )?
    .run()
    .await?;
    let known = super::who(&db, &slack, "U_GRACE", NOW).await?;
    assert_eq!(known.map(|u| u.id).as_deref(), Some("usr_grace"));
    // A match moves with her address, as when she comes back as a new
    // Slack user.
    assert!(super::may_match(&db, "usr_grace", "T1").await?);
    // Acme turns it off: fctl forgets her, and asks her to link instead.
    q(
        &db,
        "UPDATE workspace_slack SET settings = ? WHERE workspace_id = 'wsp_acme'",
        &[s(&serde_json::to_string(&SlackSettings::default())?)],
    )?
    .run()
    .await?;
    assert!(super::who(&db, &slack, "U_GRACE", NOW).await?.is_none());
    assert!(super::person(&db, "T1", "U_GRACE").await?.is_none());
    // A linked account stays whatever anyone allows.
    super::keep_account(
        &db,
        "usr_grace",
        "T1",
        "Acme",
        None,
        &grace,
        SlackVia::Linked,
        NOW,
    )
    .await?;
    let linked = super::who(&db, &slack, "U_GRACE", NOW).await?;
    assert_eq!(linked.map(|u| u.id).as_deref(), Some("usr_grace"));
    // Nor does another Slack user with her address take it over: an address
    // matches only people who linked no account there.
    assert!(!super::may_match(&db, "usr_grace", "T1").await?);
    assert!(super::may_match(&db, "usr_grace", "T2").await?);
    assert!(super::may_match(&db, "usr_ada", "T1").await?);
    Ok(())
}
