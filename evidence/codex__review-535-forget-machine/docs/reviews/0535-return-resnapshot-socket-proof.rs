
#[tokio::test]
async fn review_535_return_closes_the_socket_and_resumes_with_retained_records() {
 let st=state(None);let addr=start(Arc::clone(&st)).await;
 let (mut node,task)=node_conn(&st,"n1").await;
 assert_eq!(handshake(&mut node).await.from_seq,1);
 let bodies=[
  Body::NodeOnline(v1::NodeOnline{node_id:"n1".into(),hostname:"laptop".into(),os:"linux".into(),build:"test".into()}),
  Body::CheckoutDiscovered(v1::CheckoutDiscovered{node_id:"n1".into(),checkout_id:"c".into(),project_id:"p".into(),path:"/project".into()}),
  Body::WorkspaceRegistered(v1::WorkspaceRegistered{workspace_id:"ws".into(),node_id:"n1".into(),project_id:"p".into(),path:"/project".into(),..Default::default()}),
  Body::AgentStarted(v1::AgentStarted{agent_id:"a".into(),node_id:"n1".into(),project_id:"p".into(),kind:"shell".into(),..Default::default()}),
  Body::AgentWorkspaceBound(v1::AgentWorkspaceBound{agent_id:"a".into(),workspace_id:"ws".into()}),
 ];
 for (i,body) in bodies.into_iter().enumerate(){node.send(&f(STREAM_EVENTS,kinds::EVENT,event("n1",i as u64+1,body))).await.unwrap();}
 while next_ack(&mut node).await<5 {}
 node.close().await;task.await.unwrap();
 st.hub.forget_node("n1",false,10).unwrap();
 let mut late=dial(addr,Some("good-token")).await;
 late.send(&f(STREAM_EVENTS,kinds::SUBSCRIBE,v1::Subscribe::default().encode_to_vec())).await.unwrap();
 let first=recv(&mut late).await;assert_eq!(first.kind,kinds::SNAPSHOT);
 let hidden=v1::Snapshot::decode(first.payload.as_slice()).unwrap();
 assert!(hidden.nodes.is_empty()&&hidden.agents.is_empty()&&hidden.checkouts.is_empty()&&hidden.workspaces.is_empty());
 let (mut back,back_task)=node_conn(&st,"n1").await;
 assert_eq!(handshake(&mut back).await.from_seq,6,"node must resume without replaying old announcements");
 assert!(closed(&mut late).await,"late subscriber socket must actually close, not remain open with an empty queue");
 let mut resumed=dial(addr,Some("good-token")).await;
 resumed.send(&f(STREAM_EVENTS,kinds::SUBSCRIBE,v1::Subscribe{after_pos:Some(hidden.pos)}.encode_to_vec())).await.unwrap();
 let frame=recv(&mut resumed).await;assert_eq!(frame.kind,kinds::SNAPSHOT,"old position must receive snapshot rather than return-event replay");
 let recovered=v1::Snapshot::decode(frame.payload.as_slice()).unwrap();
 assert_eq!(recovered.nodes.len(),1);assert_eq!(recovered.nodes[0].hostname,"laptop");
 assert_eq!(recovered.agents.len(),1);assert!(recovered.agents[0].running);
 assert_eq!(recovered.checkouts.len(),1);assert_eq!(recovered.workspaces.len(),1);
 assert_eq!(recovered.agent_workspaces.len(),1);
 // The following event still crosses the snapshot/live boundary once, with the original running agent untouched.
 back.send(&f(STREAM_EVENTS,kinds::EVENT,event("n1",6,Body::NodeResources(v1::NodeResources{node_id:"n1".into(),..Default::default()})))).await.unwrap();
 let next=recv(&mut resumed).await;assert_eq!(next.kind,kinds::POSITIONED_EVENT);
 let next=v1::PositionedEvent::decode(next.payload.as_slice()).unwrap();assert_eq!(next.pos,recovered.pos+1);
 assert!(tokio::time::timeout(Duration::from_millis(100),resumed.recv()).await.is_err(),"no duplicate live event");
 assert!(st.hub.view().0.agents["n1/a"].running);
 // A fresh hub rebuilding the store must keep the resume cutoff, even after open records the node offline.
 let store=SqliteStore::open_in_memory().unwrap();
 for row in st.hub.store().events_after(0,1000).unwrap(){store.append(&row.producer,row.seq,row.at_ms,&row.payload).unwrap();}
 let reopened=Hub::open(store).unwrap();
 let again=reopened.subscribe(Some(hidden.pos)).unwrap();assert!(again.backlog.is_empty());
 let (view,pos)=again.snapshot.expect("snapshot after reopen");
 let after_restart=server::snapshot_of(&view,pos);assert_eq!(after_restart.agents.len(),1);assert_eq!(after_restart.workspaces.len(),1);
 let out=std::path::Path::new("/tmp/agora-535-fixed-wire");std::fs::create_dir_all(out).unwrap();
 std::fs::write(out.join("hidden.snapshot.bin"),first.payload).unwrap();std::fs::write(out.join("recovered.snapshot.bin"),frame.payload).unwrap();
 eprintln!("real socket closed; resume from {} gives snapshot {}; retained laptop/agent/checkout/workspace/binding; next event {} once; cutoff survives Hub::open",hidden.pos,recovered.pos,next.pos);
 back.close().await;back_task.await.unwrap();
}
