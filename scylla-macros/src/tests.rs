//! Unit tests of `authz!`: the schema text, the checks of the names, and the errors.

use proc_macro2::TokenStream;
use quote::quote;

use crate::expand::expand;

/// Returns the code of a valid input as text.
fn text(input: TokenStream) -> String {
    expand(input).expect("valid input").to_string()
}

/// Returns the code of a valid input as text with no spaces.
fn compact(input: TokenStream) -> String {
    text(input).replace(' ', "")
}

/// Returns the error messages of an input.
fn errors(input: TokenStream) -> Vec<String> {
    match expand(input) {
        Ok(_) => Vec::new(),
        Err(error) => error.into_iter().map(|error| error.to_string()).collect(),
    }
}

/// A definition of projects, as a test input.
fn projects() -> TokenStream {
    quote! {
        /// The projects.
        definition project for Project {
            /// The actor that owns the project.
            relation owner: Actor;
            relation editor: Actor;
            permission delete = owner->act_for;
            permission edit = delete + editor->act_for;
        }
    }
}

/// A definition gives its name, its schema part, one typed constant for each name, and a check
/// of each arrow on the subject type.
#[test]
fn a_definition_gives_its_part_and_its_constants() {
    // given
    let input = projects();

    // when
    let text = text(input.clone());
    let compact = compact(input);

    // then
    assert!(text.contains(r#""delete + editor->act_for""#));
    assert!(compact.contains("constNAME:&'staticstr=\"project\";"));
    assert!(compact.contains("pubconstOWNER:::scylla_domain::authz::Relation<Project,Actor>"));
    assert!(compact.contains("pubconstEDIT:::scylla_domain::authz::Permission<Project>"));
    assert!(compact.contains("let_=<Actor>::ACT_FOR;"));
}

/// A name that the definition does not have, and an arrow from a permission, are errors.
#[test]
fn a_wrong_name_in_a_permission_is_an_error() {
    // given
    let input = quote! {
        definition project for Project {
            relation owner: Actor;
            permission delete = owner->act_for;
            permission edit = delte + delete->act_for + editor->act_for;
        }
    };

    // when
    let errors = errors(input);

    // then
    assert_eq!(
        errors,
        [
            "`delte` is not a relation or a permission of `project`",
            "`delete` is a permission: an arrow starts from a relation",
            "`editor` is not a relation of `project`",
        ]
    );
}

/// A name that SpiceDB refuses, a word of the language and a name declared twice are errors.
#[test]
fn a_bad_name_is_an_error() {
    // given
    let input = quote! {
        definition Project for Project {
            relation ab: Actor;
            relation owner: Actor;
            permission owner = owner;
            permission caveat = owner;
        }
    };

    // when
    let errors = errors(input);

    // then
    assert_eq!(errors.len(), 4);
    assert!(errors[0].starts_with("`Project` is not a SpiceDB name"));
    assert!(errors[1].starts_with("`ab` is not a SpiceDB name"));
    assert_eq!(errors[2], "`owner` is declared twice");
    assert_eq!(
        errors[3],
        "`caveat` is a word of the SpiceDB schema language"
    );
}

/// An extension can use the names of its definition: the compiler checks them on its type.
#[test]
fn an_extension_checks_the_names_of_its_definition() {
    // given
    let input = quote! {
        /// An auditor of the projects.
        extend Project as ProjectAudit {
            relation auditor: Actor;
            permission audit = auditor->act_for + delete - owner->banned;
        }
    };

    // when
    let text = text(input.clone());
    let compact = compact(input);

    // then
    assert!(text.contains(r#""auditor->act_for + delete - owner->banned""#));
    assert!(compact.contains("pubstructProjectAudit;"));
    assert!(compact.contains("Part::extension(<Projectas::scylla_domain::authz::Resource>::NAME"));
    assert!(compact.contains("pubconstAUDIT:::scylla_domain::authz::Permission<Project>"));
    assert!(compact.contains("let_=<Actor>::ACT_FOR;"));
    assert!(compact.contains("let_=<Project>::DELETE;"));
    assert!(compact.contains("let_=<Project>::OWNER;"));
}

/// Parentheses and `nil` stay in the schema text.
#[test]
fn parentheses_and_nil_stay_in_the_schema() {
    // given
    let input = quote! {
        definition folder for Folder {
            relation parent: Folder;
            relation viewer: Actor;
            permission hidden = nil;
            permission view = (viewer + parent->view) & viewer - hidden;
        }
    };

    // when
    let text = text(input);

    // then
    assert!(text.contains(r#""nil""#));
    assert!(text.contains(r#""(viewer + parent->view) & viewer - hidden""#));
}

/// A block that is not a definition or an extension is an error.
#[test]
fn an_unknown_block_is_an_error() {
    // given
    let input = quote! { resource project for Project {} };

    // when
    let errors = errors(input);

    // then
    assert_eq!(errors, ["expected `definition` or `extend`"]);
}
