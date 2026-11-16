
#[tokio::test]
async fn review_535_only_an_owner_can_forget_a_machine() {
 use agora_hub::users::Role;
 for who in [Role::Member,Role::Viewer] {
  let st=Arc::new(ServerState{
   hub:Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap()),
   support:support(),ca:None,lockout:Arc::new(Lockout::new()),
   auth:Box::new(Demotable(Arc::new(std::sync::Mutex::new(who)))),assets:None,sessions:None,
  });
  let addr=start(Arc::clone(&st)).await;
  let mut client=dial(addr,Some("good-token")).await;
  client.send(&f(STREAM_EVENTS,kinds::SUBSCRIBE,v1::Subscribe::default().encode_to_vec())).await.unwrap();
  assert_eq!(recv(&mut client).await.kind,kinds::SNAPSHOT);
  let refused=forget_node(&mut client,"missing","").await;
  assert_eq!(err_code(&refused),"auth.forbidden","{who:?}: {refused:?}");
 }
}
