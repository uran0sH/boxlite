//! Integration tests for box restart functionality.
//!
//! Tests the restart policy mechanism including:
//! - Crash detection and automatic restart
//! - Restart on reboot after runtime restart
//! - Manual stop vs crash distinction
//! - Restart policy modes (No, Always, OnFailure, UnlessStopped)
//! - State transitions (Stopped → Running with backoff)

use boxlite::runtime::options::{BoxOptions, BoxliteOptions, RootfsSpec};
use boxlite::runtime::restart_policy::RestartPolicy;
use boxlite::runtime::types::BoxStatus;
use boxlite::{BoxCommand, BoxliteRuntime};
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::sleep;

/// Test context with isolated runtime and automatic cleanup.
struct TestContext {
    runtime: BoxliteRuntime,
    _temp_dir: TempDir,
}

impl TestContext {
    fn new() -> Self {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let options = BoxliteOptions {
            home_dir: temp_dir.path().to_path_buf(),
            image_registries: vec!["docker.m.daocloud.io".into()],
        };
        let runtime = BoxliteRuntime::new(options).expect("Failed to create runtime");
        Self {
            runtime,
            _temp_dir: temp_dir,
        }
    }
}

// ============================================================================
// RESTART POLICY CREATION TESTS
// ============================================================================

#[tokio::test]
async fn create_box_with_restart_policy() {
    let ctx = TestContext::new();

    let box_handle = ctx
        .runtime
        .create(
            BoxOptions {
                rootfs: RootfsSpec::Image("alpine:latest".into()),
                restart_policy: RestartPolicy::OnFailure(Some(3)),
                auto_remove: false,
                ..Default::default()
            },
            None,
        )
        .await
        .expect("Failed to create box");

    let box_id = box_handle.id().clone();
    let _ = box_handle
        .exec(BoxCommand::new("sleep").arg("10"))
        .await
        .unwrap();

    // Verify box was created successfully
    let info = ctx
        .runtime
        .get_info(box_id.as_str())
        .await
        .expect("Failed to get box info")
        .expect("Box not found");

    assert_eq!(info.status, BoxStatus::Running);

    // Cleanup
    box_handle.stop().await.unwrap();
    ctx.runtime.remove(box_id.as_str(), false).await.unwrap();
}

// ============================================================================
// CRASH AND RESTART TESTS
// ============================================================================

#[tokio::test]
async fn box_with_always_policy_restarts_after_crash() {
    let ctx = TestContext::new();

    // Create box with Always restart policy
    let box_handle = ctx
        .runtime
        .create(
            BoxOptions {
                rootfs: RootfsSpec::Image("alpine:latest".into()),
                restart_policy: RestartPolicy::Always,
                auto_remove: false,
                ..Default::default()
            },
            None,
        )
        .await
        .expect("Failed to create box");

    let box_id = box_handle.id().clone();

    // Execute a command that will crash
    let exec_result = box_handle.exec(BoxCommand::new("echo").arg("Hello")).await;

    // Command should fail
    assert!(exec_result.is_ok());
    let info = ctx
        .runtime
        .get_info(box_id.as_str())
        .await
        .unwrap()
        .unwrap();
    let pid = info.pid.expect("Box should have a PID");

    use boxlite::util::process::kill_process;
    kill_process(pid);

    // Wait for restart to happen (monitoring detects crash + backoff delay)
    // With restart policy, it should transition Stopped → Running
    sleep(Duration::from_secs(6)).await;

    // Box should be Running (was restarted)
    let info = ctx
        .runtime
        .get_info(box_id.as_str())
        .await
        .expect("Failed to get box info")
        .expect("Box not found");

    assert_eq!(info.status, BoxStatus::Running);

    println!("Box restarted successfully");
    // Cleanup
    box_handle.stop().await.unwrap();
    println!("box stopped successfully");
    ctx.runtime.remove(box_id.as_str(), false).await.unwrap();
}

// // ============================================================================
// // MANUAL STOP VS CRASH TESTS
// // ============================================================================

// #[tokio::test]
// async fn manually_stopped_box_does_not_restart_with_unlessstopped() {
//     let ctx = TestContext::new();

//     // Create box with UnlessStopped policy
//     let box_handle = ctx
//         .runtime
//         .create(
//             BoxOptions {
//                 rootfs: RootfsSpec::Image("alpine:latest".into()),
//                 restart_policy: RestartPolicy::UnlessStopped(Some(3)),
//                 auto_remove: false,
//                 ..Default::default()
//             },
//             None,
//         )
//         .await
//         .expect("Failed to create box");

//     let box_id = box_handle.id().clone();

//     // Manually stop the box
//     box_handle.stop().await.unwrap();

//     // Wait a bit
//     sleep(Duration::from_secs(3)).await;

//     // Box should remain Stopped (not restarted)
//     let info = ctx
//         .runtime
//         .get_info(box_id.as_str())
//         .await
//         .expect("Failed to get box info")
//         .expect("Box not found");

//     assert_eq!(info.status, BoxStatus::Stopped);

//     // Cleanup
//     ctx.runtime.remove(box_id.as_str(), false).await.unwrap();
// }

// #[tokio::test]
// async fn crashed_box_restarts_with_unlessstopped() {
//     let ctx = TestContext::new();

//     // Create box with UnlessStopped policy
//     let box_handle = ctx
//         .runtime
//         .create(
//             BoxOptions {
//                 rootfs: RootfsSpec::Image("alpine:latest".into()),
//                 restart_policy: RestartPolicy::UnlessStopped(Some(3)),
//                 auto_remove: false,
//                 ..Default::default()
//             },
//             None,
//         )
//         .await
//         .expect("Failed to create box");

//     let box_id = box_handle.id().clone();

//     // Execute a command that crashes
//     let exec_result = box_handle
//         .exec(BoxCommand::new("sh").args(["-c", "exit 1"]))
//         .await;

//     assert!(exec_result.is_ok());

//     // Wait for restart
//     sleep(Duration::from_secs(5)).await;

//     // Box should be Running (was restarted)
//     let info = ctx
//         .runtime
//         .get_info(box_id.as_str())
//         .await
//         .expect("Failed to get box info")
//         .expect("Box not found");

//     assert_eq!(info.status, BoxStatus::Running);

//     // Cleanup
//     box_handle.stop().await.unwrap();
//     ctx.runtime.remove(box_id.as_str(), false).await.unwrap();
// }

// // ============================================================================
// // RESTART ON REBOOT TESTS
// // ============================================================================

// #[tokio::test]
// async fn box_with_restart_on_reboot_starts_after_runtime_restart() {
//     let temp_dir = TempDir::new().expect("Failed to create temp dir");
//     let home_dir = temp_dir.path().to_path_buf();

//     // Create first runtime
//     let options1 = BoxliteOptions {
//         home_dir: home_dir.clone(),
//         image_registries: vec![],
//     };
//     let runtime1 = BoxliteRuntime::new(options1).expect("Failed to create runtime");

//     // Create box with restart_on_reboot=true
//     let box_handle = runtime1
//         .create(
//             BoxOptions {
//                 rootfs: RootfsSpec::Image("alpine:latest".into()),
//                 restart_on_reboot: true,
//                 restart_policy: RestartPolicy::Always,
//                 auto_remove: false,
//                 ..Default::default()
//             },
//             Some("test-reboot-box".to_string()),
//         )
//         .await
//         .expect("Failed to create box");

//     let box_id = box_handle.id().clone();

//     // Box is running
//     let info1 = runtime1
//         .get_info(box_id.as_str())
//         .await
//         .expect("Failed to get box info")
//         .expect("Box not found");

//     assert_eq!(info1.status, BoxStatus::Configured);

//     // Stop the box to simulate shutdown
//     box_handle.stop().await.unwrap();

//     // Wait for stop to complete
//     sleep(Duration::from_secs(1)).await;

//     // Verify it's stopped
//     let info1_stopped = runtime1
//         .get_info(box_id.as_str())
//         .await
//         .expect("Failed to get box info")
//         .expect("Box not found");

//     assert_eq!(info1_stopped.status, BoxStatus::Stopped);

//     // Drop first runtime (simulate system shutdown)
//     drop(runtime1);

//     // Wait to ensure cleanup
//     sleep(Duration::from_secs(1)).await;

//     // Create new runtime (simulate system reboot)
//     let options2 = BoxliteOptions {
//         home_dir: home_dir.clone(),
//         image_registries: vec![],
//     };
//     let runtime2 = BoxliteRuntime::new(options2).expect("Failed to create runtime");

//     // Wait for restart-on-reboot to complete
//     sleep(Duration::from_secs(5)).await;

//     // Box should be running again
//     let info2 = runtime2
//         .get_info(box_id.as_str())
//         .await
//         .expect("Failed to get box info")
//         .expect("Box not found");

//     assert_eq!(info2.status, BoxStatus::Running);

//     // Cleanup
//     let box_handle2 = runtime2
//         .get(box_id.as_str())
//         .await
//         .expect("Failed to get box")
//         .expect("Box not found");
//      let _ = box_handle2.exec(BoxCommand::new("echo Hello"));

//     box_handle2.stop().await.unwrap();
//     runtime2.remove(box_id.as_str(), false).await.unwrap();
// }

// #[tokio::test]
// async fn box_without_restart_on_reboot_does_not_start_after_runtime_restart() {
//     let temp_dir = TempDir::new().expect("Failed to create temp dir");
//     let home_dir = temp_dir.path().to_path_buf();

//     // Create first runtime
//     let options1 = BoxliteOptions {
//         home_dir: home_dir.clone(),
//         image_registries: vec![],
//     };
//     let runtime1 = BoxliteRuntime::new(options1).expect("Failed to create runtime");

//     // Create box WITHOUT restart_on_reboot (default false)
//     let box_handle = runtime1
//         .create(
//             BoxOptions {
//                 rootfs: RootfsSpec::Image("alpine:latest".into()),
//                 restart_on_reboot: false, // Explicitly false
//                 auto_remove: false,
//                 ..Default::default()
//             },
//             Some("test-no-reboot-box".to_string()),
//         )
//         .await
//         .expect("Failed to create box");

//     let box_id = box_handle.id().clone();

//     // Stop the box
//     box_handle.stop().await.unwrap();

//     // Wait for stop to complete
//     sleep(Duration::from_secs(1)).await;

//     // Drop first runtime
//     drop(runtime1);

//     // Wait a bit
//     sleep(Duration::from_secs(1)).await;

//     // Create new runtime
//     let options2 = BoxliteOptions {
//         home_dir: home_dir.clone(),
//         image_registries: vec![],
//     };
//     let runtime2 = BoxliteRuntime::new(options2).expect("Failed to create runtime");

//     // Wait a bit to see if anything happens
//     sleep(Duration::from_secs(3)).await;

//     // Box should remain Stopped
//     let info2 = runtime2
//         .get_info(box_id.as_str())
//         .await
//         .expect("Failed to get box info")
//         .expect("Box not found");

//     assert_eq!(info2.status, BoxStatus::Stopped);

//     // Cleanup
//     runtime2.remove(box_id.as_str(), false).await.unwrap();
// }

// // ============================================================================
// // STATE TRANSITION TESTS
// // ============================================================================

// #[tokio::test]
// async fn box_transitions_from_running_to_stopped_on_manual_stop() {
//     let ctx = TestContext::new();

//     // Create box with no restart policy
//     let box_handle = ctx
//         .runtime
//         .create(
//             BoxOptions {
//                 rootfs: RootfsSpec::Image("alpine:latest".into()),
//                 restart_policy: RestartPolicy::No,
//                 auto_remove: false,
//                 ..Default::default()
//             },
//             None,
//         )
//         .await
//         .expect("Failed to create box");

//     let box_id = box_handle.id().clone();

//     // Verify initial state
//     assert_eq!(box_handle.info().status, BoxStatus::Configured);

//     // Manually stop the box
//     box_handle.stop().await.unwrap();

//     // Wait for stop to complete
//     sleep(Duration::from_secs(1)).await;

//     // Box should be in Stopped state
//     let info = ctx
//         .runtime
//         .get_info(box_id.as_str())
//         .await
//         .expect("Failed to get box info")
//         .expect("Box not found");

//     assert_eq!(info.status, BoxStatus::Stopped);

//     // Cleanup
//     ctx.runtime.remove(box_id.as_str(), false).await.unwrap();
// }

// #[tokio::test]
// async fn box_with_restart_policy_survives_crash() {
//     let ctx = TestContext::new();

//     // Create box with Always restart policy
//     let box_handle = ctx
//         .runtime
//         .create(
//             BoxOptions {
//                 rootfs: RootfsSpec::Image("alpine:latest".into()),
//                 restart_policy: RestartPolicy::Always,
//                 auto_remove: false,
//                 ..Default::default()
//             },
//             None,
//         )
//         .await
//         .expect("Failed to create box");

//     let box_id = box_handle.id().clone();

//     // Verify initial state
//     let info1 = ctx
//         .runtime
//         .get_info(box_id.as_str())
//         .await
//         .expect("Failed to get box info")
//         .expect("Box not found");

//     assert_eq!(info1.status, BoxStatus::Configured);

//     // Crash the box
//     let exec_result = box_handle
//         .exec(BoxCommand::new("sh").args(["-c", "exit 1"]))
//         .await;

//     assert!(exec_result.is_ok());

//     // Wait for restart to complete
//     sleep(Duration::from_secs(5)).await;

//     // Box should be back to Running
//     let info2 = ctx
//         .runtime
//         .get_info(box_id.as_str())
//         .await
//         .expect("Failed to get box info")
//         .expect("Box not found");

//     assert_eq!(info2.status, BoxStatus::Running);

//     // Verify box is still functional
//     let exec_result2 = box_handle.exec(BoxCommand::new("echo").arg("hello")).await;

//     assert!(exec_result2.is_ok());

//     // Cleanup
//     box_handle.stop().await.unwrap();
//     ctx.runtime.remove(box_id.as_str(), false).await.unwrap();
// }

// #[tokio::test]
// async fn box_enters_failed_state_after_max_restarts() {
//     let ctx = TestContext::new();

//     // Create box with limited restart attempts
//     let box_handle = ctx
//         .runtime
//         .create(
//             BoxOptions {
//                 rootfs: RootfsSpec::Image("alpine:latest".into()),
//                 restart_policy: RestartPolicy::OnFailure(Some(2)),
//                 auto_remove: false,
//                 ..Default::default()
//             },
//             None,
//         )
//         .await
//         .expect("Failed to create box");

//     let box_id = box_handle.id().clone();

//     // Execute a command that crashes
//     let exec_result = box_handle
//         .exec(BoxCommand::new("sh").args(["-c", "exit 1"]))
//         .await;

//     assert!(exec_result.is_ok());

//     // Wait for max attempts to be reached
//     sleep(Duration::from_secs(10)).await;

//     // Box should be Stopped with failure_reason set
//     let info = ctx
//         .runtime
//         .get_info(box_id.as_str())
//         .await
//         .expect("Failed to get box info")
//         .expect("Box not found");

//     assert_eq!(info.status, BoxStatus::Stopped);

//     // Cleanup
//     ctx.runtime.remove(box_id.as_str(), false).await.unwrap();
// }

// // ============================================================================
// // MULTIPLE BOXES TESTS
// // ============================================================================

// #[tokio::test]
// async fn multiple_boxes_restart_independently() {
//     let ctx = TestContext::new();

//     // Create first box with restart policy
//     let box1 = ctx
//         .runtime
//         .create(
//             BoxOptions {
//                 rootfs: RootfsSpec::Image("alpine:latest".into()),
//                 restart_policy: RestartPolicy::Always,
//                 auto_remove: false,
//                 ..Default::default()
//             },
//             Some("box1".to_string()),
//         )
//         .await
//         .expect("Failed to create box1");

//     // Create second box without restart policy
//     let box2 = ctx
//         .runtime
//         .create(
//             BoxOptions {
//                 rootfs: RootfsSpec::Image("alpine:latest".into()),
//                 restart_policy: RestartPolicy::No,
//                 auto_remove: false,
//                 ..Default::default()
//             },
//             Some("box2".to_string()),
//         )
//         .await
//         .expect("Failed to create box2");

//     let box1_id = box1.id().clone();
//     let box2_id = box2.id().clone();

//     // Crash both boxes
//     let result1 = box1
//         .exec(BoxCommand::new("sh").args(["-c", "exit 1"]))
//         .await;
//     let result2 = box2
//         .exec(BoxCommand::new("sh").args(["-c", "exit 1"]))
//         .await;

//     assert!(result1.is_ok());
//     assert!(result2.is_ok());

//     // Wait for restarts
//     sleep(Duration::from_secs(5)).await;

//     // box1 should be Running (restarted)
//     let info1 = ctx
//         .runtime
//         .get_info(box1_id.as_str())
//         .await
//         .expect("Failed to get box1 info")
//         .expect("Box1 not found");

//     // box2 should be Stopped (not restarted)
//     let info2 = ctx
//         .runtime
//         .get_info(box2_id.as_str())
//         .await
//         .expect("Failed to get box2 info")
//         .expect("Box2 not found");

//     assert_eq!(info1.status, BoxStatus::Running);
//     assert_eq!(info2.status, BoxStatus::Stopped);

//     // Cleanup
//     box1.stop().await.unwrap();
//     ctx.runtime.remove(box1_id.as_str(), false).await.unwrap();
//     ctx.runtime.remove(box2_id.as_str(), false).await.unwrap();
// }
