//! 命令回执：空操作命令也写回执但不唤醒订阅者；非数据库错误的重放。
//!
//! 从 `lib.rs` 的 `mod tests` 拆出（`docs/08` 的 P2.3），只移动：函数体逐字不变。

use super::*;

/// 收尾的失败分支不区分错误类型：该 `command_id` 已有回执时，失败也要还原成那条回执。
///
/// 事务外的预查挡住了绝大多数重放，但两次预查之间可能已经有同一个 `command_id` 的命令落库。
/// 那种情况下输的那一次会先撞上业务守卫（例如修订已被赢的那一次推进），拿到的不是一个
/// 数据库错误，而唯一正确的结果仍是那条既有回执。
///
/// 本用例把「只认数据库错误」与「任何错误」分开钉住：把失败分支收窄回
/// `Err(StoreError::Database(error))` 后，第一次断言拿到的是那个业务错误而不是回执。
#[tokio::test]
async fn a_no_op_command_writes_its_receipt_but_does_not_wake_subscribers() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let created = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("no-op 通知", "同一模式", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    let version_id = created.version.id;
    let changed_mode = match created.workflow.approval_mode {
        ApprovalMode::AutoApprove => ApprovalMode::ApproveRequired,
        ApprovalMode::ApproveRequired => ApprovalMode::AutoApprove,
    };
    let mut notifications = store.subscribe_events();

    // 真的改了模式：命令改了东西，界面需要重读。
    let flipped = store
        .set_approval_mode(version_id, Uuid::new_v4(), 0, changed_mode)
        .await
        .unwrap();
    assert!(matches!(notifications.try_recv(), Ok(())));

    // 再写同一个模式：这条命令是 no-op，不该打扰界面。
    let no_op_command_id = Uuid::new_v4();
    let repeated = store
        .set_approval_mode(
            version_id,
            no_op_command_id,
            flipped.state_revision,
            changed_mode,
        )
        .await
        .unwrap();
    assert_eq!(repeated, flipped);
    assert!(matches!(
        notifications.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));

    // no-op 只是不广播，不是不记账：这条命令照样留下了自己的回执。
    assert!(
        entity::command_receipt::Entity::find_by_id(db_id(no_op_command_id))
            .one(&store.database)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn command_finish_replays_an_existing_receipt_for_a_non_database_error() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();

    // 先用一条真实命令落一条回执：方法、目标与指纹都取自它自己算的那一份。
    let command_id = Uuid::new_v4();
    let stored = store
        .save_system_model_settings(
            command_id,
            0,
            NewSystemModelSettings {
                api_base_url: "https://model.example/v1".to_owned(),
                api_key: CredentialPatch::Replace("secret-key-1".to_owned()),
                model_id: "writer-1".to_owned(),
                capabilities: ModelCapabilityProfile {
                    streaming: true,
                    tools: true,
                    stream_usage: true,
                    thinking: true,
                },
                max_context_tokens: 32_768,
                reserved_output_tokens: 4_096,
            },
        )
        .await
        .unwrap();
    let receipt = entity::command_receipt::Entity::find_by_id(db_id(command_id))
        .one(&store.database)
        .await
        .unwrap()
        .expect("成功命令必须留下回执");

    // 同一 commandId 再走一次收尾，事务这次以非数据库错误结束：必须返回既有回执。
    let replayed: SystemModelSettings = Command::new(
        command_id,
        &receipt.method,
        &receipt.target,
        &receipt.payload_hash,
    )
    .finish(
        &store,
        Notify::Listeners,
        Err(StoreError::SystemSettingsRevisionConflict {
            expected: 0,
            actual: 1,
        }),
    )
    .await
    .expect("已有回执时，非数据库错误也必须还原成回执");
    assert_eq!(replayed, stored);

    // 没有回执时原样返回原错误。
    let unknown: Result<SystemModelSettings, StoreError> =
        Command::new(Uuid::new_v4(), "PUT", "/api/settings", "unknown-hash")
            .finish(
                &store,
                Notify::Listeners,
                Err(StoreError::IdempotencyConflict),
            )
            .await;
    assert!(matches!(unknown, Err(StoreError::IdempotencyConflict)));

    // 有回执但指纹不同仍是真冲突，不会被这次放宽吞掉。
    let mismatched: Result<SystemModelSettings, StoreError> =
        Command::new(command_id, &receipt.method, &receipt.target, "another-hash")
            .finish(
                &store,
                Notify::Listeners,
                Err(StoreError::SystemSettingsRevisionConflict {
                    expected: 0,
                    actual: 1,
                }),
            )
            .await;
    assert!(matches!(mismatched, Err(StoreError::IdempotencyConflict)));
}
