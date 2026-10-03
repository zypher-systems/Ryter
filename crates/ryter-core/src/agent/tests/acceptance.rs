//! One session across all four hats, with deterministic provider billing.
use super::*;
use crate::user_io::{Permission, PlanAnswer, UserRequest};

fn billed(mut reply: Vec<StreamDelta>) -> Vec<StreamDelta> {
    reply.insert(
        0,
        StreamDelta::Usage(Usage {
            input_tokens: 100,
            output_tokens: 10,
            ..Default::default()
        }),
    );
    reply.insert(1, StreamDelta::ReportedCost(0.001));
    reply
}

#[tokio::test]
async fn approved_plan_build_review_test_resume_undo_and_commit_receipt() {
    let (home, repository, mut agent) = repo_setup(ReplayProvider::new(vec![]));
    let root = repository.path();
    let app = root.join("app");
    std::fs::create_dir(&app).unwrap();
    std::fs::write(app.join("greeting.txt"), "before\n").unwrap();
    std::fs::write(root.join("sibling.txt"), "sibling\n").unwrap();
    crate::git::git(root, &["add", "-A"]).unwrap();
    crate::git::git(root, &["commit", "-qm", "fixture baseline"]).unwrap();
    agent.ctx.workspace = app.clone();
    agent.project_root = Some(app.clone());
    agent.session = Session::create(home.path(), &app, "fixture".into(), "fixture".into()).unwrap();
    agent.ctx.notes_dir = agent.session.notes_dir();
    agent.budget_usd = 1.0;
    let mut cfg = crate::Config::default();
    cfg.ui.offer_audit = false;
    agent.cfg = Some(cfg);
    agent.put_on(Role::SoloPlan).unwrap();
    let (io, requests) = crate::user_io::UserIo::pair();
    agent.ctx.user_io = Some(io);
    let approvals = std::thread::spawn(move || {
        let mut seen = Vec::new();
        while let Ok(request) = requests.recv() {
            match request {
                UserRequest::Plan { plan, reply, .. } => {
                    assert!(plan.contains("greeting.txt"));
                    seen.push("plan");
                    reply.send(PlanAnswer::Approve).unwrap();
                }
                UserRequest::Permission { tool, reply, .. } => {
                    assert_eq!(tool, "audit");
                    seen.push("audit");
                    reply.send(Permission::Allow).unwrap();
                }
                UserRequest::Run { rows, reply, .. } => {
                    assert!(rows.iter().any(|(_, value)| value.contains("greeting.txt")));
                    seen.push("run");
                    reply.send(PlanAnswer::Approve).unwrap();
                }
                other => panic!("unexpected fixture approval: {other:?}"),
            }
        }
        seen
    });
    let (tx, events) = std::sync::mpsc::channel();
    agent.sink = Some(tx);
    let plan = A_PLAN.replace("README.md", "greeting.txt");
    agent.provider = Arc::new(ReplayProvider::scripted(
        vec![
            call(
                "present_plan",
                serde_json::json!({"title":"Greeting", "plan":plan}),
            ),
            write("greeting.txt", "hello reader\n"),
            say("Built the greeting."),
        ]
        .into_iter()
        .map(billed)
        .collect(),
    ));
    agent
        .turn("Plan and build the greeting; preserve sibling files.")
        .await
        .unwrap();
    assert_eq!(agent.role, Role::SoloBuild);
    assert!(agent.session.meta.plan_file.is_some());
    assert_eq!(
        std::fs::read_to_string(app.join("greeting.txt")).unwrap(),
        "hello reader\n"
    );

    agent.provider = Arc::new(ReplayProvider::scripted(vec![
        billed(call(
            "read_file",
            serde_json::json!({"path":"greeting.txt"}),
        )),
        billed(say(
            "Read greeting.txt and compared it with the approved plan.\nVERDICT: PASS",
        )),
    ]));
    assert!(agent.review_now().await.unwrap().contains("VERDICT: PASS"));
    agent.put_on(Role::SoloBuild).unwrap();
    agent.provider = Arc::new(ReplayProvider::scripted(vec![
        billed(call(
            "propose_run",
            serde_json::json!({
                "test":["test \"$(cat greeting.txt)\" = 'hello reader'"]
            }),
        )),
        billed(run_project("test")),
        billed(say("Testing complete: greeting contents as approved.")),
    ]));
    agent
        .turn("Check the greeting through the approved run file and report.")
        .await
        .unwrap();
    assert!(matches!(
        crate::run::find(&app, home.path()),
        crate::run::Found::Approved(_)
    ));
    assert!(
        agent
            .session
            .transcript
            .iter()
            .any(|m| m.content.contains("greeting contents"))
    );
    let spend = agent.session.spend_log().unwrap();
    assert_eq!(spend.len(), 8);
    assert!(
        spend
            .iter()
            .all(|row| !row.incomplete && row.total_usd == Some(0.001))
    );
    let total = agent.session.meta.spend_usd_total.unwrap();
    assert!((total - 0.008).abs() < 1e-10);

    // Resume both conversations, then undo/redo while preserving a later user edit.
    agent.ctx.user_io = None;
    assert_eq!(approvals.join().unwrap(), ["plan", "audit", "run"]);
    agent.session = Session::open(&agent.session.dir).unwrap();
    assert_eq!(agent.session.spend_log().unwrap().len(), spend.len());
    agent.put_on(Role::SoloBuild).unwrap();
    std::fs::write(app.join("user-note.txt"), "keep my later edit\n").unwrap();
    std::fs::write(root.join("sibling.txt"), "later sibling edit\n").unwrap();
    agent.undo().unwrap();
    assert_eq!(
        std::fs::read_to_string(app.join("greeting.txt")).unwrap(),
        "before\n"
    );
    agent.redo().unwrap();
    assert_eq!(
        std::fs::read_to_string(app.join("greeting.txt")).unwrap(),
        "hello reader\n"
    );
    assert_eq!(
        std::fs::read_to_string(app.join("user-note.txt")).unwrap(),
        "keep my later edit\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("sibling.txt")).unwrap(),
        "later sibling edit\n"
    );
    let events: Vec<_> = events.try_iter().collect();
    assert!(events.iter().any(|e| matches!(
        e,
        AgentEvent::Reviewed {
            verdict: Some(true),
            ..
        }
    )));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, AgentEvent::ToolResult { is_error: true, .. }))
    );
    // Later user changes invalidate a review/test mark for the current whole tree.
    let changes = crate::review::changes(&app, &crate::review::head_base(&app)).unwrap();
    let now = crate::review::tree_of(&app, &changes.now);
    let review_mark = events.iter().find_map(|e| match e {
        AgentEvent::Reviewed {
            tree,
            model,
            verdict,
            ..
        } => Some((tree.clone(), model.clone(), *verdict)),
        _ => None,
    });
    let receipt = crate::review::Receipt {
        models: vec![agent.model.clone()],
        usd: total,
        review: crate::review::Reviewed::of(review_mark.as_ref(), now.as_deref()),
        ..Default::default()
    };
    assert_eq!(receipt.review, crate::review::Reviewed::Stale);
    let message = crate::review::with_receipt("Add greeting", &receipt);
    crate::review::commit(&app, &["app/greeting.txt".into()], &message).unwrap();
    let committed = crate::git::git(root, &["log", "-1", "--format=%B"]).unwrap();
    assert!(
        committed.contains("Ryter:") && committed.contains("not reviewed after the last change")
    );
    assert_eq!(
        crate::git::git(root, &["show", "HEAD:app/greeting.txt"]).unwrap(),
        "hello reader\n"
    );
    assert_eq!(
        crate::git::git(root, &["show", "HEAD:sibling.txt"]).unwrap(),
        "sibling\n"
    );
}
