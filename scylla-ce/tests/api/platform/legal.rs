//! The tests of `LegalService`: anyone reads the legal settings, only an instance admin changes
//! them, and the account mails link the legal documents.

use scylla_proto::platform::v1::legal_document::Content;
use scylla_proto::platform::v1::{
    LegalDocument, LegalDocumentKind, LegalOperator, UpdateLegalSettingsRequest,
};
use sqlx::PgPool;
use tonic::Code;

use crate::{app, assert_code, assert_invalid_field};

/// Makes a document of a kind.
fn document(kind: LegalDocumentKind, content: Option<Content>) -> LegalDocument {
    LegalDocument {
        kind: kind as i32,
        content,
        ..LegalDocument::default()
    }
}

/// Makes a request that changes the documents only.
fn documents(documents: Vec<LegalDocument>) -> UpdateLegalSettingsRequest {
    UpdateLegalSettingsRequest {
        documents,
        ..UpdateLegalSettingsRequest::default()
    }
}

/// A new instance has no legal document, no operator and no privacy contact, and anyone reads
/// this with no token.
#[sqlx::test(migrations = false)]
async fn get_legal_info_needs_no_token(pool: PgPool) {
    // given
    let app = app(pool).await;

    // when
    let info = app.get_legal_info().await.unwrap();

    // then
    assert!(info.documents.is_empty());
    assert_eq!(info.operator.unwrap(), LegalOperator::default());
    assert_eq!(info.privacy_contact, "");
}

/// An instance admin sets the operator, the privacy contact and the documents. Each document
/// replaces the document of its kind, a document with no content turns its kind off, and the
/// list follows the order of the kinds. The text of what changes keeps its lines.
#[sqlx::test(migrations = false)]
async fn update_legal_settings_changes_the_documents(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let operator = LegalOperator {
        name: "Acme SAS".to_owned(),
        address: "1 rue de la Paix\n75002 Paris".to_owned(),
        ..LegalOperator::default()
    };
    let terms = LegalDocument {
        title: " Terms ".to_owned(),
        version: "2.0".to_owned(),
        effective_date: "2027-01-01".to_owned(),
        acceptance_required: true,
        changes: "\n- Export of your data.\n- Clearer rules for runners.\n".to_owned(),
        ..document(
            LegalDocumentKind::Terms,
            Some(Content::Markdown("# Terms\n".to_owned())),
        )
    };
    let privacy = document(
        LegalDocumentKind::Privacy,
        Some(Content::Url("https://acme.example/privacy".to_owned())),
    );
    let first = UpdateLegalSettingsRequest {
        operator: Some(operator.clone()),
        privacy_contact: Some("privacy@acme.example".to_owned()),
        documents: vec![terms, privacy],
    };

    // when
    app.update_legal_settings(admin, first).await.unwrap();
    let off = documents(vec![document(LegalDocumentKind::Privacy, None)]);
    let changed = app.update_legal_settings(admin, off).await.unwrap();
    let info = app.get_legal_info().await.unwrap();

    // then
    assert_eq!(info, changed);
    assert_eq!(info.operator.unwrap(), operator);
    assert_eq!(info.privacy_contact, "privacy@acme.example");
    let kinds: Vec<i32> = info.documents.iter().map(|doc| doc.kind).collect();
    assert_eq!(kinds, [LegalDocumentKind::Terms as i32]);
    let terms = &info.documents[0];
    assert_eq!(terms.title, "Terms");
    assert_eq!(terms.version, "2.0");
    assert_eq!(terms.effective_date, "2027-01-01");
    assert!(terms.acceptance_required && terms.updated_at.is_some());
    assert_eq!(
        terms.changes,
        "- Export of your data.\n- Clearer rules for runners."
    );
    assert_eq!(
        terms.content,
        Some(Content::Markdown("# Terms\n".to_owned()))
    );
}

/// Only a caller with the route permission changes the legal settings.
#[sqlx::test(migrations = false)]
async fn update_legal_settings_needs_the_route_permission(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let notice = document(
        LegalDocumentKind::LegalNotice,
        Some(Content::Markdown(String::new())),
    );

    // when
    let refused = app
        .update_legal_settings(user, documents(vec![notice]))
        .await;

    // then
    assert_code(refused, Code::PermissionDenied);
    assert!(app.get_legal_info().await.unwrap().documents.is_empty());
}

/// A bad value gives `INVALID_ARGUMENT` with the path of its field, and changes nothing.
#[sqlx::test(migrations = false)]
async fn update_legal_settings_refuses_a_bad_value(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let url = |url: &str| Some(Content::Url(url.to_owned()));
    let dated = |date: &str| LegalDocument {
        effective_date: date.to_owned(),
        ..document(LegalDocumentKind::Dpa, url("https://acme.example/dpa"))
    };
    let cookies = document(LegalDocumentKind::Cookies, url("https://acme.example"));
    let changes = |text: String| LegalDocument {
        changes: text,
        ..document(LegalDocumentKind::Terms, url("https://acme.example/terms"))
    };
    let cases = [
        (
            documents(vec![document(LegalDocumentKind::Privacy, url("ftp://x"))]),
            "documents[0].url",
        ),
        (
            documents(vec![dated("2027-02-30")]),
            "documents[0].effective_date",
        ),
        (
            documents(vec![cookies.clone(), cookies]),
            "documents[1].kind",
        ),
        (
            documents(vec![changes("a".repeat(2001))]),
            "documents[0].changes",
        ),
        (
            documents(vec![changes("a\u{0}b".to_owned())]),
            "documents[0].changes",
        ),
        (
            documents(vec![document(
                LegalDocumentKind::Unspecified,
                url("https://a"),
            )]),
            "documents[0].kind",
        ),
        (
            UpdateLegalSettingsRequest {
                operator: Some(LegalOperator {
                    name: "A\nB".to_owned(),
                    ..LegalOperator::default()
                }),
                ..UpdateLegalSettingsRequest::default()
            },
            "operator.name",
        ),
    ];

    for (request, field) in cases {
        // when
        let status = app.update_legal_settings(admin, request).await.unwrap_err();

        // then
        assert_invalid_field(&status, field);
    }
    let info = app.get_legal_info().await.unwrap();
    assert!(info.documents.is_empty() && info.operator.unwrap().name.is_empty());
}

/// A mail links the privacy policy and the legal notice that are on, a text of the web app at its
/// page, and names the operator in its footer.
#[sqlx::test(migrations = false)]
async fn a_mail_links_the_legal_documents(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let request = UpdateLegalSettingsRequest {
        operator: Some(LegalOperator {
            name: "Acme SAS".to_owned(),
            address: "1 rue de la Paix\n75002 Paris".to_owned(),
            ..LegalOperator::default()
        }),
        privacy_contact: None,
        documents: vec![
            document(
                LegalDocumentKind::Privacy,
                Some(Content::Url("https://acme.example/privacy".to_owned())),
            ),
            document(
                LegalDocumentKind::LegalNotice,
                Some(Content::Markdown("Text".to_owned())),
            ),
        ],
    };
    app.update_legal_settings(admin, request).await.unwrap();
    let email = "zoe@example.com";

    // when
    app.create_invitation(owner, workspace, email, "viewer")
        .await
        .unwrap();

    // then
    let mail = app.mails.to(email).pop().unwrap();
    let notice = "http://127.0.0.1:3000/legal/legal-notice";
    assert!(mail
        .body
        .contains("Privacy policy: https://acme.example/privacy\n"));
    assert!(mail.body.contains(&format!("Legal notice: {notice}\n")));
    assert!(mail
        .body
        .contains("Acme SAS · 1 rue de la Paix · 75002 Paris\n"));
    assert!(mail.html.contains(r#"href="https://acme.example/privacy""#));
    assert!(mail.html.contains(&format!(r#"href="{notice}""#)));
}

/// A mail of an instance with no legal document has no legal link in its footer.
#[sqlx::test(migrations = false)]
async fn a_mail_has_no_legal_link_by_default(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let email = "ada@example.com";

    // when
    app.create_invitation(owner, workspace, email, "viewer")
        .await
        .unwrap();

    // then
    let mail = app.mails.to(email).pop().unwrap();
    assert!(!mail.body.contains("Privacy policy") && !mail.html.contains("Legal notice"));
}
