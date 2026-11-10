//! 配置事务的顺序与恢复规则；原生副作用通过同一个接口执行和测试。
pub(crate) trait Effects {
    async fn prepare(&mut self) -> Result<(), String>;
    async fn commit(&mut self) -> Result<(), String>;
    async fn rollback(&mut self) -> Vec<String>;
}

pub(crate) async fn run(effects: &mut impl Effects) -> Result<(), String> {
    effects.prepare().await?;
    if let Err(error) = effects.commit().await {
        let failures = effects.rollback().await;
        return Err(if failures.is_empty() {
            error
        } else {
            format!(
                "{error}；恢复原状态未完成：{}。请核对网络状态后重试",
                failures.join("；")
            )
        });
    }
    Ok(())
}

/// Committed configuration is authoritative even when protection cannot be re-armed.
/// Callers hold the configuration lock across commit and this completion step.
pub(crate) trait CompletionEffects {
    fn config_changed(&mut self);
    async fn rearm(&mut self) -> Result<(), String>;
}

pub(crate) fn needs_rearm(strict_before: bool, armed_before: bool, runtime_changed: bool) -> bool {
    strict_before && armed_before && runtime_changed
}

pub(crate) async fn finish_committed(
    effects: &mut impl CompletionEffects,
    rearm: bool,
) -> Result<(), String> {
    // Do not hide a committed value when a later protection check fails.
    effects.config_changed();
    if rearm {
        effects.rearm().await.map_err(|error| {
            format!("配置已保存，但严格保护尚未就绪：{error}。阻断继续保留，请重新启用严格保护")
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        browser_open: bool,
        fail_save: bool,
        fail_rollback: bool,
        core: u16,
        proxy: u16,
        events: Vec<&'static str>,
    }
    impl Default for Fixture {
        fn default() -> Self {
            Self {
                browser_open: false,
                fail_save: false,
                fail_rollback: false,
                core: 7890,
                proxy: 7890,
                events: vec![],
            }
        }
    }
    impl Effects for Fixture {
        async fn prepare(&mut self) -> Result<(), String> {
            self.events.push("prepare");
            if self.browser_open {
                Err("请关闭指纹浏览器".into())
            } else {
                Ok(())
            }
        }
        async fn commit(&mut self) -> Result<(), String> {
            self.events.push("core");
            self.core = 7891;
            self.events.push("proxy");
            self.proxy = 7891;
            self.events.push("save");
            if self.fail_save {
                Err("保存配置失败".into())
            } else {
                Ok(())
            }
        }
        async fn rollback(&mut self) -> Vec<String> {
            self.events.push("rollback");
            if self.fail_rollback {
                vec!["内核恢复失败".into(), "系统代理恢复失败".into()]
            } else {
                self.core = 7890;
                self.proxy = 7890;
                vec![]
            }
        }
    }
    // 夹具的 Future 无 IO，不需要启动异步运行时或真实网络。
    fn execute(fixture: &mut Fixture) -> Result<(), String> {
        let mut future = std::pin::pin!(run(fixture));
        match std::future::Future::poll(
            future.as_mut(),
            &mut std::task::Context::from_waker(std::task::Waker::noop()),
        ) {
            std::task::Poll::Ready(result) => result,
            std::task::Poll::Pending => panic!("fixture unexpectedly waited for IO"),
        }
    }
    #[test]
    fn browser_precondition_failure_has_no_side_effects() {
        let mut f = Fixture {
            browser_open: true,
            ..Default::default()
        };
        assert!(execute(&mut f).unwrap_err().contains("指纹浏览器"));
        assert_eq!(f.events, ["prepare"]);
        assert_eq!((f.core, f.proxy), (7890, 7890));
    }
    #[test]
    fn saving_failure_restores_core_and_proxy() {
        let mut f = Fixture {
            fail_save: true,
            ..Default::default()
        };
        assert_eq!(execute(&mut f).unwrap_err(), "保存配置失败");
        assert_eq!(f.events, ["prepare", "core", "proxy", "save", "rollback"]);
        assert_eq!((f.core, f.proxy), (7890, 7890));
    }
    #[test]
    fn rollback_failures_are_returned_alongside_original_failure() {
        let mut f = Fixture {
            fail_save: true,
            fail_rollback: true,
            ..Default::default()
        };
        let error = execute(&mut f).unwrap_err();
        for detail in ["保存配置失败", "内核恢复失败", "系统代理恢复失败"] {
            assert!(error.contains(detail));
        }
    }
    #[test]
    fn successful_commit_does_not_rollback() {
        let mut f = Fixture::default();
        execute(&mut f).unwrap();
        assert_eq!(f.events, ["prepare", "core", "proxy", "save"]);
        assert_eq!((f.core, f.proxy), (7891, 7891));
    }

    struct CompletionFixture {
        fail_rearm: bool,
        events: Vec<&'static str>,
    }
    impl CompletionEffects for CompletionFixture {
        fn config_changed(&mut self) {
            self.events.push("config-changed");
        }
        async fn rearm(&mut self) -> Result<(), String> {
            self.events.push("verify-and-arm");
            if self.fail_rearm {
                Err("TUN 未就绪".into())
            } else {
                Ok(())
            }
        }
    }
    fn complete(fixture: &mut CompletionFixture, rearm: bool) -> Result<(), String> {
        let mut future = std::pin::pin!(finish_committed(fixture, rearm));
        match std::future::Future::poll(
            future.as_mut(),
            &mut std::task::Context::from_waker(std::task::Waker::noop()),
        ) {
            std::task::Poll::Ready(result) => result,
            std::task::Poll::Pending => panic!("completion fixture unexpectedly waited for IO"),
        }
    }
    #[test]
    fn only_previously_armed_strict_runtime_changes_request_rearm() {
        for strict in [false, true] {
            for armed in [false, true] {
                for runtime_changed in [false, true] {
                    let mut fixture = CompletionFixture {
                        fail_rearm: false,
                        events: vec![],
                    };
                    complete(&mut fixture, needs_rearm(strict, armed, runtime_changed)).unwrap();
                    let expected = if strict && armed && runtime_changed {
                        vec!["config-changed", "verify-and-arm"]
                    } else {
                        vec!["config-changed"]
                    };
                    assert_eq!(fixture.events, expected);
                }
            }
        }
    }
    #[test]
    fn failed_rearm_notifies_committed_state_before_returning_failure() {
        let mut fixture = CompletionFixture {
            fail_rearm: true,
            events: vec![],
        };
        let error = complete(&mut fixture, true).unwrap_err();
        assert_eq!(fixture.events, ["config-changed", "verify-and-arm"]);
        assert!(error.contains("配置已保存"));
        assert!(error.contains("TUN 未就绪"));
        assert!(error.contains("阻断继续保留"));
    }
}
