use super::*;
use crate::application::pagination::{PaginatedResult, PaginationParams};
use crate::domain::ids::UserId;
use crate::domain::user::User;
use crate::test_support::stubs::StubUsers;
use crate::test_support::users::user;

#[tokio::test]
async fn users_in_order_keeps_the_page_order_and_metadata_and_drops_unknown_ids() {
    let (bob, carol) = (user("bob"), user("carol"));
    let users = StubUsers::with([bob.clone(), carol.clone()]);
    let ids = vec![bob.id().clone(), UserId::new("ghost"), carol.id().clone()];
    let page = PaginatedResult::new(ids, &PaginationParams::default(), 7);

    let users = users_in_order(&users, page).await.unwrap();

    let got: Vec<&UserId> = users.items().iter().map(User::id).collect();
    assert_eq!(got, vec![bob.id(), carol.id()]);
    assert_eq!(users.metadata().total_count(), 7);
}
