//! 凭据存取测试：确认 Windows 凭据管理器的系统调用真的能用
//! （结构体布局、写入、读回、覆盖、删除都要走通）。
//!
//! 故意使用带进程号的目标名，避免覆盖真正保存的 DeepSeek 密钥。

#[cfg(target_os = "windows")]
#[test]
fn credential_round_trip_uses_the_system_vault() {
    use crate::secret_store;

    let target = format!("Q-IDE/test/{}", std::process::id());

    // 先清掉上次测试异常退出可能留下的记录。
    secret_store::delete(&target).expect("清理旧测试凭据");
    assert_eq!(secret_store::load(&target), None);

    secret_store::save(&target, "sk-test-1234567890").expect("写入凭据");
    assert_eq!(
        secret_store::load(&target).as_deref(),
        Some("sk-test-1234567890")
    );

    // 同名再写一次只保留最新值。
    secret_store::save(&target, "sk-test-updated").expect("覆盖写入凭据");
    assert_eq!(
        secret_store::load(&target).as_deref(),
        Some("sk-test-updated")
    );

    secret_store::delete(&target).expect("删除凭据");
    assert_eq!(secret_store::load(&target), None);
    // 已经不存在的凭据再删一次不算失败。
    secret_store::delete(&target).expect("重复删除不报错");
}
