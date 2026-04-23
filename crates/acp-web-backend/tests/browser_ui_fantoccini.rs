#[allow(dead_code, unused_imports)]
#[path = "session_api_roundtrip/support.rs"]
mod support;

use std::{
    env,
    ffi::OsString,
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use chrono::NaiveDateTime;
use fantoccini::{Client, ClientBuilder, Locator, key::Key, wd::Capabilities};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::process::{Child, Command};

use support::{ServerConfig, TestStack, test_state_dir};

const APP_PATH: &str = "/app/";
const REGISTER_PATH: &str = "/app/register/";
const WORKSPACES_PATH: &str = "/app/workspaces/";
const COMPOSER_SELECTOR: &str = "#composer-input";
const REGISTER_USERNAME_SELECTOR: &str = ".account-form input[type='text']";
const REGISTER_PASSWORD_SELECTOR: &str = ".account-form input[type='password']";
const SUBMIT_SELECTOR: &str = ".composer__submit";
const SIDEBAR_TOGGLE_SELECTOR: &str = ".session-sidebar__toggle";
const MOCK_REPLY_TEXT: &str = "mock assistant: I received test.";
const WEBDRIVER_READY_ATTEMPTS: usize = 50;
const WEBDRIVER_READY_DELAY: Duration = Duration::from_millis(100);
const WEBDRIVER_START_RETRIES: usize = 5;

#[tokio::test]
#[ignore = "requires ChromeDriver, Chrome, and a built frontend bundle"]
async fn slash_prefix_can_be_removed_without_breaking_prompt_submission() -> Result<()> {
    let browser = BrowserHarness::spawn((1280, 960)).await?;
    let result = async {
        browser.open_app().await?;
        browser.run_slash_prefix_submission("test").await?;

        Ok(())
    }
    .await;

    browser.shutdown().await;
    result
}

#[tokio::test]
#[ignore = "requires ChromeDriver, Chrome, and a built frontend bundle"]
async fn sidebar_shows_activity_metadata_and_closed_state() -> Result<()> {
    let browser = BrowserHarness::spawn((1280, 960)).await?;
    let result = async {
        browser.open_app().await?;
        browser.ensure_sidebar_visible().await?;
        let (activity_label, status_label) = browser.session_sidebar_metadata().await?;

        assert_eq!(status_label, "active");
        assert!(activity_label.starts_with("Updated "));
        assert!(activity_label.ends_with(" UTC"));
        let timestamp = activity_label
            .strip_prefix("Updated ")
            .and_then(|value| value.strip_suffix(" UTC"))
            .context("sidebar activity label did not match the expected shape")?;
        NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d %H:%M")
            .context("parsing the sidebar activity timestamp")?;

        browser.close_current_session().await?;

        browser
            .wait_for_condition(
                "return document.querySelector('.session-sidebar__status-pill')\
                 ?.textContent?.trim() === 'closed';",
                Duration::from_secs(10),
                "closed status pill",
            )
            .await?;
        browser
            .wait_for_body_text("This conversation has ended.", Duration::from_secs(10))
            .await?;

        Ok(())
    }
    .await;

    browser.shutdown().await;
    result
}

#[tokio::test]
#[ignore = "requires ChromeDriver, Chrome, and a built frontend bundle"]
async fn sidebar_shows_current_workspace_label() -> Result<()> {
    let browser = BrowserHarness::spawn((1280, 960)).await?;
    let result = async {
        browser.open_app().await?;
        browser.ensure_sidebar_visible().await?;

        assert_eq!(
            browser.session_sidebar_workspace_label().await?,
            "Workspace: Default workspace"
        );

        Ok(())
    }
    .await;

    browser.shutdown().await;
    result
}

#[tokio::test]
#[ignore = "requires ChromeDriver, Chrome, and a built frontend bundle"]
async fn workspaces_page_is_reachable_via_sidebar_link() -> Result<()> {
    let browser = BrowserHarness::spawn((1280, 960)).await?;
    let result = async {
        browser.open_app().await?;
        browser.ensure_sidebar_visible().await?;

        browser.click_workspaces_link().await?;

        browser
            .wait_for_condition(
                &format!(
                    "return window.location.pathname === {WORKSPACES_PATH:?} \
                     || window.location.pathname === '/app/workspaces';",
                ),
                Duration::from_secs(10),
                "workspaces page navigation",
            )
            .await?;

        browser
            .wait_for_condition(
                "return Boolean(document.querySelector('h1')) \
                 && document.querySelector('h1')?.textContent?.trim() === 'Workspaces';",
                Duration::from_secs(10),
                "workspaces page heading",
            )
            .await?;

        Ok(())
    }
    .await;

    browser.shutdown().await;
    result
}

#[tokio::test]
#[ignore = "requires ChromeDriver, Chrome, and a built frontend bundle"]
async fn workspaces_page_can_create_update_and_delete_workspace() -> Result<()> {
    let browser = BrowserHarness::spawn((1280, 960)).await?;
    let result = async {
        browser.open_app().await?;
        browser.navigate_to_workspaces().await?;

        let workspace_name = "Browser-Test Workspace";
        let renamed = "Browser-Test Workspace Renamed";
        browser.create_workspace_and_confirm(workspace_name).await?;
        browser
            .rename_workspace_and_confirm(workspace_name, renamed)
            .await?;
        browser.delete_workspace_and_confirm(renamed).await?;

        Ok(())
    }
    .await;

    browser.shutdown().await;
    result
}

struct BrowserHarness {
    client: Client,
    stack: TestStack,
    webdriver: WebDriverProcess,
}

impl BrowserHarness {
    async fn spawn(viewport: (u32, u32)) -> Result<Self> {
        let frontend_dist = frontend_dist_path()?;
        let stack = TestStack::spawn(ServerConfig {
            session_cap: 8,
            acp_server: String::new(),
            startup_hints: false,
            state_dir: test_state_dir(),
            frontend_dist: Some(frontend_dist),
        })
        .await
        .context("spawning the browser test backend stack")?;

        let webdriver = WebDriverProcess::spawn().await?;
        let client = match connect_browser(&webdriver.endpoint, viewport).await {
            Ok(client) => client,
            Err(error) => {
                webdriver.shutdown().await;
                return Err(error);
            }
        };

        Ok(Self {
            client,
            stack,
            webdriver,
        })
    }

    async fn open_app(&self) -> Result<()> {
        self.client
            .goto(&format!("{}{}", self.stack.backend_url, APP_PATH))
            .await
            .context("opening the browser app shell")?;

        if self
            .wait_for_optional_condition(
                &format!("return window.location.pathname === {REGISTER_PATH:?};"),
                Duration::from_secs(5),
            )
            .await?
        {
            self.complete_bootstrap_registration().await?;
        }

        self.wait_for_condition(
            "return Boolean(document.querySelector('#composer-input'));",
            Duration::from_secs(30),
            "composer bootstrap",
        )
        .await?;
        self.wait_for_condition(
            r#"return /\/app\/sessions\/[^/]+$/.test(window.location.pathname);"#,
            Duration::from_secs(30),
            "browser session route",
        )
        .await
    }

    async fn complete_bootstrap_registration(&self) -> Result<()> {
        let username = self
            .client
            .find(Locator::Css(REGISTER_USERNAME_SELECTOR))
            .await
            .context("finding the bootstrap username input")?;
        username
            .send_keys("admin")
            .await
            .context("typing the bootstrap username")?;
        let password = self
            .client
            .find(Locator::Css(REGISTER_PASSWORD_SELECTOR))
            .await
            .context("finding the bootstrap password input")?;
        password
            .send_keys("password123")
            .await
            .context("typing the bootstrap password")?;
        self.client
            .find(Locator::Css(".account-form__submit"))
            .await
            .context("finding the bootstrap submit button")?
            .click()
            .await
            .context("submitting the bootstrap registration form")?;
        self.wait_for_condition(
            r#"return /\/app\/sessions\/[^/]+$/.test(window.location.pathname);"#,
            Duration::from_secs(30),
            "bootstrap registration redirect",
        )
        .await
    }

    async fn ensure_sidebar_visible(&self) -> Result<()> {
        let is_visible: bool = self
            .evaluate(
                "const node = document.querySelector('.session-sidebar'); \
                 return Boolean(node) && getComputedStyle(node).display !== 'none';",
                "checking sidebar visibility",
            )
            .await?;
        if is_visible {
            return Ok(());
        }

        self.client
            .find(Locator::Css(SIDEBAR_TOGGLE_SELECTOR))
            .await
            .context("finding the sidebar toggle")?
            .click()
            .await
            .context("opening the sidebar")?;
        self.wait_for_condition(
            "const node = document.querySelector('.session-sidebar'); \
             return Boolean(node) && getComputedStyle(node).display !== 'none';",
            Duration::from_secs(10),
            "visible session sidebar",
        )
        .await
    }

    async fn focused_composer(&self) -> Result<fantoccini::elements::Element> {
        let composer = self
            .client
            .find(Locator::Css(COMPOSER_SELECTOR))
            .await
            .context("finding the composer textarea")?;
        composer.click().await.context("focusing the composer")?;
        Ok(composer)
    }

    async fn open_browser_help_palette(&self) -> Result<fantoccini::elements::Element> {
        let composer = self.focused_composer().await?;
        composer.send_keys("/").await.context("typing slash")?;
        self.wait_for_slash_palette().await?;
        Ok(composer)
    }

    async fn wait_for_slash_palette(&self) -> Result<()> {
        self.wait_for_condition(
            "return Boolean(document.querySelector('.composer__slash-palette')) \
             && document.querySelectorAll('.composer__slash-item').length > 0;",
            Duration::from_secs(10),
            "slash command palette",
        )
        .await
    }

    async fn assert_browser_help_only_palette(&self) -> Result<()> {
        let item_texts = self.slash_palette_items().await?;
        assert!(item_texts.iter().any(|text| text.contains("/help")));
        assert!(!item_texts.iter().any(|text| text.contains("/cancel")));
        assert!(!item_texts.iter().any(|text| text.contains("/approve")));
        assert!(!item_texts.iter().any(|text| text.contains("/deny")));
        assert!(!item_texts.iter().any(|text| text.contains("/quit")));
        Ok(())
    }

    async fn slash_palette_items(&self) -> Result<Vec<String>> {
        self.evaluate(
            "return Array.from(document.querySelectorAll('.composer__slash-item'))\
             .map((item) => item.textContent.trim());",
            "reading slash command labels",
        )
        .await
    }

    async fn run_slash_prefix_submission(&self, prompt: &str) -> Result<()> {
        let composer = self.open_browser_help_palette().await?;
        self.assert_browser_help_only_palette().await?;
        self.remove_slash_prefix(&composer).await?;
        self.enter_prompt(&composer, prompt).await?;
        self.assert_composer_submission_ready().await?;
        self.click_submit_button().await?;
        self.wait_for_body_text(MOCK_REPLY_TEXT, Duration::from_secs(30))
            .await?;
        self.assert_empty_composer().await
    }

    async fn remove_slash_prefix(&self, composer: &fantoccini::elements::Element) -> Result<()> {
        composer
            .send_keys(&Key::Backspace.to_string())
            .await
            .context("deleting the slash prefix")?;
        self.wait_for_condition(
            "return document.querySelector('#composer-input')?.value === '';",
            Duration::from_secs(10),
            "empty composer after removing slash",
        )
        .await
    }

    async fn enter_prompt(
        &self,
        composer: &fantoccini::elements::Element,
        prompt: &str,
    ) -> Result<()> {
        composer
            .send_keys(prompt)
            .await
            .context("typing a normal prompt")
    }

    async fn click_submit_button(&self) -> Result<()> {
        self.client
            .find(Locator::Css(SUBMIT_SELECTOR))
            .await
            .context("finding the submit button")?
            .click()
            .await
            .context("submitting the prompt")
    }

    async fn assert_empty_composer(&self) -> Result<()> {
        let composer_value: String = self
            .evaluate(
                "return document.querySelector('#composer-input')?.value ?? '';",
                "reading composer value after submit",
            )
            .await?;
        assert_eq!(composer_value, "");
        Ok(())
    }

    async fn assert_composer_submission_ready(&self) -> Result<()> {
        let composer_disabled: bool = self
            .evaluate(
                "return document.querySelector('#composer-input')?.disabled ?? true;",
                "checking composer enabled state",
            )
            .await?;
        let submit_disabled: bool = self
            .evaluate(
                "return document.querySelector('.composer__submit')?.disabled ?? true;",
                "checking submit enabled state",
            )
            .await?;
        ensure!(
            !composer_disabled,
            "composer should stay enabled after removing slash"
        );
        ensure!(
            !submit_disabled,
            "submit button should stay enabled after removing slash"
        );
        Ok(())
    }

    async fn session_sidebar_metadata(&self) -> Result<(String, String)> {
        self.wait_for_condition(
            "return Boolean(document.querySelector('.session-sidebar__session-activity')) \
             && Boolean(document.querySelector('.session-sidebar__status-pill'));",
            Duration::from_secs(10),
            "sidebar metadata",
        )
        .await?;

        let activity_label = self
            .evaluate(
                "return document.querySelector('.session-sidebar__session-activity')\
                 ?.textContent?.trim() ?? '';",
                "reading session activity label",
            )
            .await?;
        let status_label = self
            .evaluate(
                "return document.querySelector('.session-sidebar__status-pill')\
                 ?.textContent?.trim() ?? '';",
                "reading session status label",
            )
            .await?;
        Ok((activity_label, status_label))
    }

    async fn session_sidebar_workspace_label(&self) -> Result<String> {
        self.wait_for_condition(
            "return document.querySelector('.session-sidebar__workspace')\
             ?.textContent?.trim()?.length > 0;",
            Duration::from_secs(10),
            "sidebar workspace label",
        )
        .await?;

        self.evaluate(
            "return document.querySelector('.session-sidebar__workspace')\
             ?.textContent?.trim() ?? '';",
            "reading workspace label",
        )
        .await
    }

    async fn click_workspaces_link(&self) -> Result<()> {
        self.wait_for_condition(
            "return Boolean(document.querySelector('a[href=\"/app/workspaces/\"]'));",
            Duration::from_secs(10),
            "workspaces sidebar link",
        )
        .await?;
        self.client
            .find(Locator::Css("a[href='/app/workspaces/']"))
            .await
            .context("finding the workspaces sidebar link")?
            .click()
            .await
            .context("clicking the workspaces sidebar link")
    }

    async fn navigate_to_workspaces(&self) -> Result<()> {
        self.client
            .goto(&format!("{}{}", self.stack.backend_url, WORKSPACES_PATH))
            .await
            .context("navigating to the workspaces page")?;
        self.wait_for_condition(
            "return Boolean(document.querySelector('h1')) \
             && document.querySelector('h1')?.textContent?.trim() === 'Workspaces';",
            Duration::from_secs(15),
            "workspaces page heading after direct navigation",
        )
        .await
    }

    async fn create_workspace(&self, name: &str) -> Result<()> {
        self.wait_for_condition(
            "return Boolean(document.querySelector('.account-form input[type=\"text\"]'));",
            Duration::from_secs(10),
            "workspace name input",
        )
        .await?;
        let input = self
            .client
            .find(Locator::Css(".account-form input[type='text']"))
            .await
            .context("finding workspace name input")?;
        input
            .click()
            .await
            .context("focusing workspace name input")?;
        input
            .send_keys(name)
            .await
            .context("typing workspace name")?;
        self.client
            .find(Locator::Css(".account-form__submit"))
            .await
            .context("finding create workspace submit button")?
            .click()
            .await
            .context("submitting the create workspace form")
    }

    async fn create_workspace_and_confirm(&self, name: &str) -> Result<()> {
        self.create_workspace(name).await?;
        self.wait_for_workspace_notice("Workspace created.").await?;
        self.wait_for_body_text(name, Duration::from_secs(10)).await
    }

    async fn rename_workspace(&self, current_name: &str, new_name: &str) -> Result<()> {
        self.open_workspace_rename(current_name).await?;
        self.clear_workspace_name_input().await?;
        self.workspace_name_input()
            .await?
            .send_keys(new_name)
            .await
            .context("typing new workspace name")?;
        self.click_workspace_save_button().await
    }

    async fn rename_workspace_and_confirm(&self, current_name: &str, new_name: &str) -> Result<()> {
        self.rename_workspace(current_name, new_name).await?;
        self.wait_for_workspace_notice("Workspace updated.").await?;
        self.wait_for_body_text(new_name, Duration::from_secs(10))
            .await
    }

    async fn delete_workspace(&self, name: &str) -> Result<()> {
        self.wait_for_condition(
            &format!(
                "return Array.from(document.querySelectorAll('.workspace-action-btn--danger'))\
                 .some(btn => btn.closest('tr')?.textContent?.includes({name:?}));"
            ),
            Duration::from_secs(10),
            "delete button for workspace",
        )
        .await?;

        self.client
            .execute(
                &format!(
                    "const btn = Array.from(document.querySelectorAll('.workspace-action-btn--danger'))\
                     .find(b => b.closest('tr')?.textContent?.includes({name:?})); \
                     if (btn) btn.click();"
                ),
                Vec::new(),
            )
            .await
            .context("clicking delete button for workspace")?;

        Ok(())
    }

    async fn delete_workspace_and_confirm(&self, name: &str) -> Result<()> {
        self.delete_workspace(name).await?;
        self.wait_for_workspace_notice("Workspace deleted.").await?;
        self.wait_for_body_text_to_disappear(name, Duration::from_secs(10))
            .await
    }

    async fn wait_for_workspace_notice(&self, notice: &str) -> Result<()> {
        self.wait_for_body_text(notice, Duration::from_secs(10))
            .await
    }

    async fn open_workspace_rename(&self, current_name: &str) -> Result<()> {
        self.wait_for_workspace_action_button(
            ".workspace-action-btn",
            current_name,
            "Rename",
            "rename button",
        )
        .await?;
        self.click_workspace_action_button(".workspace-action-btn", current_name, "Rename")
            .await?;
        self.wait_for_condition(
            "return Boolean(document.querySelector('.workspace-name-input'));",
            Duration::from_secs(10),
            "workspace name edit input",
        )
        .await
    }

    async fn wait_for_workspace_action_button(
        &self,
        selector: &str,
        row_name: &str,
        button_label: &str,
        description: &str,
    ) -> Result<()> {
        let script = workspace_action_button_script(selector, row_name, button_label);
        self.wait_for_condition(&script, Duration::from_secs(10), description)
            .await
    }

    async fn click_workspace_action_button(
        &self,
        selector: &str,
        row_name: &str,
        button_label: &str,
    ) -> Result<()> {
        let script = workspace_action_button_click_script(selector, row_name, button_label);
        self.client
            .execute(&script, Vec::new())
            .await
            .with_context(|| format!("clicking {button_label} button for workspace {row_name}"))?;
        Ok(())
    }

    async fn workspace_name_input(&self) -> Result<fantoccini::elements::Element> {
        self.client
            .find(Locator::Css(".workspace-name-input"))
            .await
            .context("finding workspace name edit input")
    }

    async fn clear_workspace_name_input(&self) -> Result<()> {
        self.client
            .execute(
                "document.querySelector('.workspace-name-input').value = '';",
                Vec::new(),
            )
            .await
            .context("clearing the workspace name input")?;
        Ok(())
    }

    async fn click_workspace_save_button(&self) -> Result<()> {
        self.client
            .find(Locator::Css(
                ".workspace-name-input + .workspace-action-btn",
            ))
            .await
            .context("finding Save button")?
            .click()
            .await
            .context("clicking Save button")
    }

    async fn close_current_session(&self) -> Result<()> {
        let close_result = self.close_current_session_request().await?;
        ensure_close_current_session_succeeded(&close_result)
    }

    async fn close_current_session_request(&self) -> Result<Value> {
        self.client
            .execute_async(close_current_session_script(), Vec::new())
            .await
            .context("closing the session from the browser")
    }

    async fn wait_for_body_text(&self, needle: &str, timeout: Duration) -> Result<()> {
        let encoded = serde_json::to_string(needle).context("encoding body-text needle")?;
        self.wait_for_condition(
            &format!("return document.body?.innerText?.includes({encoded}) ?? false;"),
            timeout,
            &format!("body text containing {needle}"),
        )
        .await
    }

    async fn wait_for_body_text_to_disappear(&self, needle: &str, timeout: Duration) -> Result<()> {
        let encoded = serde_json::to_string(needle).context("encoding body-text needle")?;
        self.wait_for_condition(
            &format!("return !(document.body?.innerText?.includes({encoded}) ?? false);"),
            timeout,
            &format!("body text excluding {needle}"),
        )
        .await
    }

    async fn wait_for_condition(
        &self,
        script: &str,
        timeout: Duration,
        description: &str,
    ) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.evaluate::<bool>(script, description).await {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(error) if Instant::now() < deadline => drop(error),
                Err(error) => return Err(error).context(description.to_string()),
            }

            if Instant::now() >= deadline {
                bail!("timed out waiting for {description}");
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn wait_for_optional_condition(&self, script: &str, timeout: Duration) -> Result<bool> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.evaluate::<bool>(script, "optional condition").await {
                Ok(true) => return Ok(true),
                Ok(false) => {}
                Err(_) if Instant::now() < deadline => {}
                Err(error) => return Err(error),
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn evaluate<T>(&self, script: &str, description: &str) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let value = self
            .client
            .execute(script, Vec::new())
            .await
            .with_context(|| format!("executing browser script for {description}"))?;
        serde_json::from_value(value)
            .with_context(|| format!("decoding browser script result for {description}"))
    }

    async fn shutdown(self) {
        let BrowserHarness {
            client,
            stack: _stack,
            webdriver,
        } = self;

        let _ = client.close().await;
        webdriver.shutdown().await;
    }
}

fn close_current_session_script() -> &'static str {
    r#"
    const callback = arguments[arguments.length - 1];
    const sessionId = window.location.pathname.split("/").pop();
    const csrfToken = document
        .querySelector("meta[name='acp-csrf-token']")
        ?.getAttribute("content") ?? "";
    fetch(`/api/v1/sessions/${encodeURIComponent(sessionId)}/close`, {
        method: "POST",
        headers: { "x-csrf-token": csrfToken },
    })
        .then(async (response) => {
            if (!response.ok) {
                callback({
                    ok: false,
                    status: response.status,
                    body: await response.text(),
                });
                return;
            }
            callback({ ok: true });
        })
        .catch((error) => callback({ ok: false, error: String(error) }));
    "#
}

fn ensure_close_current_session_succeeded(close_result: &Value) -> Result<()> {
    let close_payload = close_result
        .as_object()
        .context("close response was not an object")?;
    ensure!(
        close_payload.get("ok").and_then(Value::as_bool) == Some(true),
        "browser close request failed: {close_result}"
    );
    Ok(())
}

fn workspace_action_button_script(selector: &str, row_name: &str, button_label: &str) -> String {
    format!(
        "return Array.from(document.querySelectorAll({selector:?}))\
         .some(btn => btn.closest('tr')?.textContent?.includes({row_name:?}) \
                 && btn.textContent?.trim() === {button_label:?});"
    )
}

fn workspace_action_button_click_script(
    selector: &str,
    row_name: &str,
    button_label: &str,
) -> String {
    format!(
        "const btn = Array.from(document.querySelectorAll({selector:?}))\
         .find(candidate => candidate.closest('tr')?.textContent?.includes({row_name:?}) \
                 && candidate.textContent?.trim() === {button_label:?}); \
         if (btn) btn.click();"
    )
}

struct WebDriverProcess {
    endpoint: String,
    child: Child,
}

impl WebDriverProcess {
    async fn spawn() -> Result<Self> {
        let chromedriver_bin =
            env::var_os("ACP_CHROMEDRIVER_BIN").unwrap_or_else(|| OsString::from("chromedriver"));
        let mut last_error = None;

        for _ in 0..WEBDRIVER_START_RETRIES {
            let port = reserve_local_port().context("reserving a ChromeDriver port")?;
            let endpoint = format!("http://127.0.0.1:{port}");

            let mut child = Command::new(&chromedriver_bin)
                .arg(format!("--port={port}"))
                .arg("--allowed-ips=127.0.0.1")
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .with_context(|| {
                    format!(
                        "spawning ChromeDriver from {}",
                        PathBuf::from(&chromedriver_bin).display()
                    )
                })?;

            match wait_for_webdriver_ready(&mut child, &endpoint).await {
                Ok(()) => return Ok(Self { endpoint, child }),
                Err(error) => {
                    last_error = Some(error);
                    if child.id().is_some() {
                        let _ = child.start_kill();
                    }
                    let _ = child.wait().await;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            anyhow::anyhow!("starting ChromeDriver failed after retrying ephemeral ports")
        }))
    }

    async fn shutdown(mut self) {
        if self.child.id().is_some() {
            let _ = self.child.start_kill();
        }
        let _ = self.child.wait().await;
    }
}

async fn wait_for_webdriver_ready(child: &mut Child, endpoint: &str) -> Result<()> {
    let address = endpoint.trim_start_matches("http://");
    for _ in 0..WEBDRIVER_READY_ATTEMPTS {
        if let Ok(stream) = std::net::TcpStream::connect(address) {
            drop(stream);
            return Ok(());
        }

        if let Some(status) = child
            .try_wait()
            .context("checking whether ChromeDriver exited early")?
        {
            bail!("ChromeDriver exited before it became ready: {status}");
        }

        tokio::time::sleep(WEBDRIVER_READY_DELAY).await;
    }

    bail!("timed out waiting for ChromeDriver at {endpoint}")
}

fn frontend_dist_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os("ACP_WEB_FRONTEND_DIST") {
        let path = PathBuf::from(path);
        ensure!(
            path.exists(),
            "ACP_WEB_FRONTEND_DIST does not exist: {}",
            path.display()
        );
        return Ok(path);
    }

    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../acp-web-frontend")
        .join("dist");
    ensure!(
        path.exists(),
        "frontend bundle not found at {}; run `cd crates/acp-web-frontend && trunk build --release` or set ACP_WEB_FRONTEND_DIST",
        path.display()
    );
    Ok(path)
}

fn reserve_local_port() -> Result<u16> {
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").context("binding an ephemeral port")?;
    let port = listener
        .local_addr()
        .context("reading the ephemeral port")?
        .port();
    drop(listener);
    Ok(port)
}

async fn connect_browser(webdriver_url: &str, viewport: (u32, u32)) -> Result<Client> {
    let mut capabilities = Capabilities::new();
    capabilities.insert("browserName".to_string(), json!("chrome"));
    capabilities.insert("acceptInsecureCerts".to_string(), json!(true));
    capabilities.insert("pageLoadStrategy".to_string(), json!("eager"));

    let mut chrome_options = json!({
        "args": [
            "--headless=new",
            "--no-sandbox",
            "--disable-dev-shm-usage",
            "--disable-gpu",
            "--allow-insecure-localhost",
            "--ignore-certificate-errors",
            format!("--window-size={},{}", viewport.0, viewport.1),
        ],
    });
    if let Some(binary) = env::var_os("ACP_CHROME_BINARY") {
        chrome_options["binary"] = Value::String(PathBuf::from(binary).display().to_string());
    }
    capabilities.insert("goog:chromeOptions".to_string(), chrome_options);

    let mut builder =
        ClientBuilder::rustls().context("building the Fantoccini Rustls connector")?;
    builder.capabilities(capabilities);
    builder
        .connect(webdriver_url)
        .await
        .context("connecting Fantoccini to ChromeDriver")
}
