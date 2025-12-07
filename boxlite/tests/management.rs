//! Integration tests for box lifecycle management.

use boxlite::BoxliteRuntime;
use boxlite::management::BoxState;
use boxlite::runtime::options::{BoxOptions, BoxliteOptions};
use boxlite_shared::Transport;
use tempfile::TempDir;

/// Helper to create a test runtime with a temporary home directory
fn create_test_runtime() -> (BoxliteRuntime, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let options = BoxliteOptions {
        home_dir: temp_dir.path().to_path_buf(),
    };
    let runtime = BoxliteRuntime::new(options).expect("Failed to create runtime");
    (runtime, temp_dir)
}

#[test]
fn test_runtime_initialization() {
    let (runtime, _temp_dir) = create_test_runtime();
    assert!(runtime.list().unwrap().is_empty());
}

#[tokio::test]
async fn test_box_id_generation() {
    let (runtime, _temp_dir) = create_test_runtime();
    let (_id1, _box1) = runtime.create(BoxOptions::default()).unwrap();
    let (_id2, _box2) = runtime.create(BoxOptions::default()).unwrap();

    // IDs should be unique
    assert_ne!(_id1, _id2);

    // IDs should be 26 characters (ULID format)
    assert_eq!(_id1.len(), 26);
    assert_eq!(_id2.len(), 26);

    // Cleanup
    _box1.shutdown().await.unwrap();
    _box2.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_list_boxes() {
    let (runtime, _temp_dir) = create_test_runtime();

    // Initially empty
    assert_eq!(runtime.list().unwrap().len(), 0);

    // Create two boxes
    let (id1, box1) = runtime.create(BoxOptions::default()).unwrap();
    let (id2, box2) = runtime.create(BoxOptions::default()).unwrap();

    // List should show both boxes
    let boxes = runtime.list().unwrap();
    assert_eq!(boxes.len(), 2);

    let ids: Vec<&str> = boxes.iter().map(|b| b.id.as_str()).collect();
    assert!(ids.contains(&id1.as_str()));
    assert!(ids.contains(&id2.as_str()));

    // Verify both are in Starting or Running state
    for info in &boxes {
        assert!(
            info.state == BoxState::Starting || info.state == BoxState::Running,
            "Expected Starting or Running, got {:?}",
            info.state
        );
    }

    // Cleanup
    box1.shutdown().await.unwrap();
    box2.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_box_handle_id() {
    let (runtime, _temp_dir) = create_test_runtime();
    let (box_id, handle) = runtime.create(BoxOptions::default()).unwrap();

    // Handle should know its ID
    assert_eq!(handle.id(), &box_id);

    // Cleanup
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_box_handle_info() {
    let (runtime, _temp_dir) = create_test_runtime();
    let (box_id, handle) = runtime.create(BoxOptions::default()).unwrap();

    // Get info from handle
    let info = handle.info().unwrap();
    assert_eq!(info.id, box_id);
    assert!(
        info.state == BoxState::Starting || info.state == BoxState::Running,
        "Expected Starting or Running, got {:?}",
        info.state
    );
    assert_eq!(info.cpus, 4); // Default value
    assert_eq!(info.memory_mib, 4096); // Default value

    // Cleanup
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_get_info_by_id() {
    let (runtime, _temp_dir) = create_test_runtime();
    let (box_id, handle) = runtime.create(BoxOptions::default()).unwrap();

    // Get info from runtime
    let info = runtime.get(&box_id).unwrap().unwrap();
    assert_eq!(info.id, box_id);
    assert!(
        info.state == BoxState::Starting || info.state == BoxState::Running,
        "Expected Starting or Running, got {:?}",
        info.state
    );

    // Non-existent ID should return None
    let missing = runtime.get(&"nonexistent-id".to_string()).unwrap();
    assert!(missing.is_none());

    // Cleanup
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_remove_box() {
    let (runtime, _temp_dir) = create_test_runtime();
    let (box_id, handle) = runtime.create(BoxOptions::default()).unwrap();

    // Cannot remove running box
    assert!(runtime.remove(&box_id).is_err());

    // Shutdown first
    handle.shutdown().await.unwrap();

    // Now removal should succeed
    runtime.remove(&box_id).unwrap();

    // Box should no longer exist
    assert!(runtime.get(&box_id).unwrap().is_none());
}

#[tokio::test]
async fn test_box_metadata() {
    let options = BoxOptions {
        cpus: Some(4),
        memory_mib: Some(1024),
        ..Default::default()
    };

    let (runtime, _temp_dir) = create_test_runtime();
    let (box_id, handle) = runtime.create(options).unwrap();

    let info = runtime.get(&box_id).unwrap().unwrap();

    // Verify metadata was stored correctly
    assert_eq!(info.cpus, 4);
    assert_eq!(info.memory_mib, 1024);
    assert!(info.created_at.timestamp() > 0);
    // Verify transport is Unix socket
    match info.transport {
        Transport::Unix { socket_path } => {
            assert!(!socket_path.as_os_str().is_empty());
        }
        _ => panic!("Expected Unix transport"),
    }

    // Cleanup
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_state_transitions() {
    let (runtime, _temp_dir) = create_test_runtime();
    let (box_id, handle) = runtime.create(BoxOptions::default()).unwrap();

    // Box should be Starting or Running after creation
    let info = runtime.get(&box_id).unwrap().unwrap();
    assert!(
        info.state == BoxState::Starting || info.state == BoxState::Running,
        "Expected Starting or Running, got {:?}",
        info.state
    );

    // Shutdown
    handle.shutdown().await.unwrap();

    // Should now be Stopped
    let info = runtime.get(&box_id).unwrap().unwrap();
    assert_eq!(info.state, BoxState::Stopped);
}

#[tokio::test]
async fn test_multiple_runtimes_isolated() {
    let (runtime1, _temp_dir1) = create_test_runtime();
    let (runtime2, _temp_dir2) = create_test_runtime();

    let (id1, box1) = runtime1.create(BoxOptions::default()).unwrap();
    let (id2, box2) = runtime2.create(BoxOptions::default()).unwrap();

    // Each runtime should only see its own box
    assert_eq!(runtime1.list().unwrap().len(), 1);
    assert_eq!(runtime2.list().unwrap().len(), 1);

    assert_eq!(runtime1.list().unwrap()[0].id, id1);
    assert_eq!(runtime2.list().unwrap()[0].id, id2);

    // Cleanup
    box1.shutdown().await.unwrap();
    box2.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_sorting_by_creation_time() {
    let (runtime, _temp_dir) = create_test_runtime();

    // Create boxes with small delay to ensure different timestamps
    let (id1, box1) = runtime.create(BoxOptions::default()).unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    let (id2, box2) = runtime.create(BoxOptions::default()).unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    let (id3, box3) = runtime.create(BoxOptions::default()).unwrap();

    // List should be sorted newest first
    let boxes = runtime.list().unwrap();
    assert_eq!(boxes.len(), 3);
    assert_eq!(boxes[0].id, id3); // Newest
    assert_eq!(boxes[1].id, id2);
    assert_eq!(boxes[2].id, id1); // Oldest

    // Cleanup
    box1.shutdown().await.unwrap();
    box2.shutdown().await.unwrap();
    box3.shutdown().await.unwrap();
}
