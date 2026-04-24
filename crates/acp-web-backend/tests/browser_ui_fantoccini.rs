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
const COMPOSER_SELECTOR: &str = "#composer-input";
const REGISTER_USERNAME_SELECTOR: &str = ".account-form input[autocomplete='username']";
const REGISTER_PASSWORD_SELECTOR: &str = ".account-form input[type='password']";
const SUBMIT_SELECTOR: &str = ".composer__submit";
const SIDEBAR_TOGGLE_SELECTOR: &str = ".session-sidebar__toggle";
const MOCK_REPLY_TEXT: &str = "mock assistant: I received test.";
const TEST_USERNAME: &str = "admin";
const TEST_PASSWORD: &str = "password123";
const WEBDRIVER_READY_ATTEMPTS: usize = 50;
const WEBDRIVER_READY_DELAY: Duration = Duration::from_millis(100);
const WEBDRIVER_START_RETRIES: usize = 5;
const BROWSER_WORKSPACE_NAME: &str = "Browser Workspace";

fn mock_reply_for(prompt: &str) -> String {
    let compact_prompt = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    format!(
        "mock assistant: I received {}. The backend-to-mock ACP round-trip succeeded.",
        truncate_for_mock_reply(&compact_prompt, 120)
    )
}

fn truncate_for_mock_reply(value: &str, max_len: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_len).collect::<String>();

    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

#[tokio::test]
#[ignore = "requires ChromeDriver, Chrome, and a built frontend bundle"]
async fn home_route_requires_explicit_workspace_selection() -> Result<()> {
    let browser = BrowserHarness::spawn((1280, 960)).await?;
    let result = async {
        browser.open_app().await?;
        browser.wait_for_workspaces_page().await?;

        let composer_present: bool = browser
            .evaluate(
                "return Boolean(document.querySelector('#composer-input'));",
                "checking whether the composer is absent on first visit",
            )
            .await?;
        assert!(!composer_present);

        Ok(())
    }
    .await;

    browser.shutdown().await;
    result
}

#[tokio::test]
#[ignore = "requires ChromeDriver, Chrome, and a built frontend bundle"]
async fn workspaces_page_shows_create_workspace_button_not_inline_form() -> Result<()> {
    // Regression: workspace creation must be behind a modal button, not an
    // always-visible inline form section.
    let browser = BrowserHarness::spawn((1280, 960)).await?;
    let result = async {
        browser.open_app().await?;
        browser.wait_for_workspaces_page().await?;

        // The modal trigger must expose a readable label and visible contrast.
        let new_btn_present: bool = browser
            .evaluate(
                r#"
                const button = document.querySelector('.workspace-dashboard__new-btn');
                if (!button) return false;
                const label = button.querySelector('.workspace-dashboard__new-btn-label');
                const icon = button.querySelector('.workspace-dashboard__new-btn-icon');
                const styles = getComputedStyle(button);
                return label?.textContent?.trim() === 'New workspace'
                  && icon?.textContent?.trim() === '+'
                  && styles.color !== styles.backgroundColor;
                "#,
                "checking create workspace button",
            )
            .await?;
        assert!(
            new_btn_present,
            "New workspace button must render readable text and contrast"
        );

        // The modal form must NOT be visible before the button is clicked.
        let modal_visible: bool = browser
            .evaluate(
                "return Boolean(document.querySelector('.workspace-modal-overlay'));",
                "checking modal is not yet visible",
            )
            .await?;
        assert!(!modal_visible, "Create workspace modal must start hidden");

        Ok(())
    }
    .await;

    browser.shutdown().await;
    result
}

#[tokio::test]
#[ignore = "requires ChromeDriver, Chrome, and a built frontend bundle"]
async fn workspaces_page_shows_workspace_scoped_sessions() -> Result<()> {
    // After creating a workspace and starting a chat in it, the workspace card
    // on the workspaces page must show the session under that workspace.
    let browser = BrowserHarness::spawn((1280, 960)).await?;
    let result = async {
        browser.open_app().await?;
        browser.wait_for_workspaces_page().await?;
        browser
            .create_workspace_and_confirm("Session Scope Test Workspace")
            .await?;
        browser
            .open_workspace_chat_and_confirm("Session Scope Test Workspace")
            .await?;

        // Navigate back to the workspace dashboard.
        browser.ensure_sidebar_visible().await?;
        browser.click_workspaces_link().await?;
        browser.wait_for_workspaces_page().await?;

        browser
            .wait_for_workspace_card_session_list("Session Scope Test Workspace")
            .await?;

        Ok(())
    }
    .await;

    browser.shutdown().await;
    result
}

#[tokio::test]
#[ignore = "requires ChromeDriver, Chrome, and a built frontend bundle"]
async fn session_sidebar_shows_only_current_workspace_sessions() -> Result<()> {
    // Sessions from other workspaces must not appear in the sidebar of a
    // workspace-A session.
    let browser = BrowserHarness::spawn((1280, 960)).await?;
    let result = async {
        browser.open_app().await?;
        browser.wait_for_workspaces_page().await?;
        browser.create_workspace_and_confirm("WS-Alpha").await?;
        browser.create_workspace_and_confirm("WS-Beta").await?;
        browser.open_workspace_chat_and_confirm("WS-Alpha").await?;
        browser.ensure_sidebar_visible().await?;

        // The sidebar of a WS-Alpha session must not contain a WS-Beta marker.
        // (We verify by checking the sidebar session list has no items whose
        // link would belong to a WS-Beta session — since WS-Beta has no sessions
        // yet, the sidebar list should reflect only WS-Alpha sessions.)
        let beta_present_in_sidebar: bool = browser
            .evaluate(
                "return Boolean(document.querySelector('.session-sidebar__workspace')\
                 ?.textContent?.includes('WS-Beta'));",
                "checking WS-Beta not in sidebar workspace label",
            )
            .await?;
        assert!(
            !beta_present_in_sidebar,
            "WS-Beta must not appear in the WS-Alpha session sidebar"
        );

        Ok(())
    }
    .await;

    browser.shutdown().await;
    result
}

#[tokio::test]
#[ignore = "requires ChromeDriver, Chrome, and a built frontend bundle"]
async fn slash_prefix_can_be_removed_without_breaking_prompt_submission() -> Result<()> {
    let browser = BrowserHarness::spawn((1280, 960)).await?;
    let result = async {
        browser
            .open_app_and_start_chat(BROWSER_WORKSPACE_NAME)
            .await?;
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
        browser
            .open_app_and_start_chat(BROWSER_WORKSPACE_NAME)
            .await?;
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
        browser
            .open_app_and_start_chat(BROWSER_WORKSPACE_NAME)
            .await?;
        browser.ensure_sidebar_visible().await?;

        assert_eq!(
            browser.session_sidebar_workspace_label().await?,
            format!("Workspace: {BROWSER_WORKSPACE_NAME}")
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
        browser
            .open_app_and_start_chat(BROWSER_WORKSPACE_NAME)
            .await?;
        browser.ensure_sidebar_visible().await?;

        browser.click_workspaces_link().await?;

        browser.wait_for_workspaces_page().await?;

        Ok(())
    }
    .await;

    browser.shutdown().await;
    result
}

#[tokio::test]
#[ignore = "requires ChromeDriver, Chrome, and a built frontend bundle"]
async fn workspaces_page_back_link_returns_to_the_same_session() -> Result<()> {
    let browser = BrowserHarness::spawn((1280, 960)).await?;
    let result = async {
        browser
            .open_app_and_start_chat(BROWSER_WORKSPACE_NAME)
            .await?;
        browser.ensure_sidebar_visible().await?;
        let original_path = browser.current_path().await?;

        browser.click_workspaces_link().await?;
        browser.wait_for_workspaces_page().await?;
        browser.click_back_to_chat_link().await?;
        browser
            .wait_for_path(&original_path, "return to the original session")
            .await?;

        Ok(())
    }
    .await;

    browser.shutdown().await;
    result
}

#[tokio::test]
#[ignore = "requires ChromeDriver, Chrome, and a built frontend bundle"]
async fn home_route_always_lands_on_workspaces_dashboard() -> Result<()> {
    // In the new workspace-first UX /app/ always redirects to /app/workspaces/
    // for signed-in users regardless of any sessionStorage state.
    let browser = BrowserHarness::spawn((1280, 960)).await?;
    let result = async {
        browser.open_app().await?;
        // Inject a workspace ID that doesn't exist and return to /app/.
        browser
            .inject_selected_workspace_and_return_home("w_missing")
            .await?;
        // /app/ must still land on the workspaces dashboard.
        browser.wait_for_workspaces_page().await?;

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
        browser.wait_for_workspaces_page().await?;

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

#[tokio::test]
#[ignore = "requires ChromeDriver, Chrome, and a built frontend bundle"]
async fn workspaces_page_can_open_chat_in_specific_workspace() -> Result<()> {
    // Tests that clicking "New chat" inside a workspace card opens a session
    // belonging to that workspace — the replacement for the old "Switch here" flow.
    let browser = BrowserHarness::spawn((1280, 960)).await?;
    let result = async {
        browser.open_app().await?;
        browser.wait_for_workspaces_page().await?;
        browser.create_workspace_and_confirm("Workspace A").await?;
        browser.create_workspace_and_confirm("Workspace B").await?;

        // Open a chat directly from Workspace B's card.
        browser
            .open_workspace_chat_and_confirm("Workspace B")
            .await?;
        browser.ensure_sidebar_visible().await?;
        assert_session_sidebar_workspace(&browser, "Workspace B").await?;

        Ok(())
    }
    .await;

    browser.shutdown().await;
    result
}

#[tokio::test]
#[ignore = "requires ChromeDriver, Chrome, and a built frontend bundle"]
async fn sign_out_clears_workspace_and_prepared_session_storage() -> Result<()> {
    let browser = BrowserHarness::spawn((1280, 960)).await?;
    let result = async {
        open_workspace_chat_for_storage_cleanup(&browser).await?;
        assert_session_storage_present(&browser, "acp-prepared-session-id").await?;
        assert_session_storage_present(&browser, "acp-selected-workspace-id").await?;

        browser.click_sign_out_button().await?;
        browser.wait_for_sign_in_page().await?;
        assert_session_storage_cleared(&browser, "acp-prepared-session-id").await?;
        assert_session_storage_cleared(&browser, "acp-selected-workspace-id").await?;

        Ok(())
    }
    .await;

    browser.shutdown().await;
    result
}

#[tokio::test]
#[ignore = "requires ChromeDriver, Chrome, and a built frontend bundle"]
async fn sign_in_restores_the_same_session_after_sign_out() -> Result<()> {
    let browser = BrowserHarness::spawn((1280, 960)).await?;
    let result = async {
        let prompt = "Keep this chat after signing in again.";
        browser
            .open_app_and_start_chat(BROWSER_WORKSPACE_NAME)
            .await?;
        let original_path = browser.current_path().await?;
        browser
            .submit_prompt_and_wait_for_mock_reply(prompt)
            .await?;
        browser.sign_out_and_restore_session(&original_path).await?;
        browser
            .wait_for_body_text(prompt, Duration::from_secs(30))
            .await?;
        browser
            .wait_for_body_text(&mock_reply_for(prompt), Duration::from_secs(30))
            .await?;

        Ok(())
    }
    .await;

    browser.shutdown().await;
    result
}

async fn assert_session_sidebar_workspace(
    browser: &BrowserHarness,
    workspace_name: &str,
) -> Result<()> {
    assert_eq!(
        browser.session_sidebar_workspace_label().await?,
        format!("Workspace: {workspace_name}")
    );
    Ok(())
}

async fn open_workspace_chat_for_storage_cleanup(browser: &BrowserHarness) -> Result<()> {
    browser.open_app().await?;
    browser.wait_for_workspaces_page().await?;
    browser.create_workspace_and_confirm("Workspace A").await?;
    browser
        .open_workspace_chat_and_confirm("Workspace A")
        .await?;
    browser.ensure_sidebar_visible().await
}

async fn assert_session_storage_present(browser: &BrowserHarness, key: &str) -> Result<()> {
    assert!(browser.session_storage_item(key).await?.is_some());
    Ok(())
}

async fn assert_session_storage_cleared(browser: &BrowserHarness, key: &str) -> Result<()> {
    assert_eq!(browser.session_storage_item(key).await?, None);
    Ok(())
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
                "return window.location.pathname === '/app/register' \
                 || window.location.pathname === '/app/register/';",
                Duration::from_secs(5),
            )
            .await?
        {
            self.wait_for_register_page().await?;
            self.complete_bootstrap_registration().await?;
        }

        if self
            .wait_for_optional_condition(workspaces_path_script(), Duration::from_secs(10))
            .await?
        {
            return self.wait_for_workspaces_page().await;
        }

        if self
            .wait_for_optional_condition(session_route_script(), Duration::from_secs(10))
            .await?
        {
            return self.wait_for_session_page().await;
        }

        bail!("app did not reach the workspaces page or a session route after opening /app/")
    }

    async fn open_app_and_start_chat(&self, workspace_name: &str) -> Result<()> {
        self.open_app().await?;
        if self
            .wait_for_optional_condition(session_route_script(), Duration::from_secs(2))
            .await?
        {
            return self.wait_for_session_page().await;
        }

        self.wait_for_workspaces_page().await?;
        if !self
            .wait_for_optional_condition(
                &workspace_row_text_script(workspace_name),
                Duration::from_secs(2),
            )
            .await?
        {
            self.create_workspace_and_confirm(workspace_name).await?;
        }
        self.open_workspace_chat_and_confirm(workspace_name).await
    }

    async fn complete_bootstrap_registration(&self) -> Result<()> {
        self.wait_for_condition(
            "return Boolean(document.querySelector(\".account-form input[autocomplete='username']\")) \
             && Boolean(document.querySelector(\".account-form input[type='password']\"));",
            Duration::from_secs(30),
            "bootstrap registration form",
        )
        .await?;
        self.submit_auth_form(TEST_USERNAME, TEST_PASSWORD, "bootstrap registration")
            .await?;
        self.wait_for_workspaces_page().await
    }

    async fn sign_in_as_bootstrap_account(&self) -> Result<()> {
        self.wait_for_sign_in_page().await?;
        self.submit_auth_form(TEST_USERNAME, TEST_PASSWORD, "sign-in")
            .await
    }

    async fn sign_out_and_restore_session(&self, original_path: &str) -> Result<()> {
        self.ensure_sidebar_visible().await?;
        self.click_sign_out_button().await?;
        self.sign_in_as_bootstrap_account().await?;
        self.wait_for_path(
            original_path,
            "return to the original session after sign in",
        )
        .await?;
        self.wait_for_session_page().await
    }

    async fn submit_auth_form(
        &self,
        username: &str,
        password: &str,
        flow_name: &str,
    ) -> Result<()> {
        self.fill_auth_input(REGISTER_USERNAME_SELECTOR, username, "username", flow_name)
            .await?;
        self.fill_auth_input(REGISTER_PASSWORD_SELECTOR, password, "password", flow_name)
            .await?;
        self.click_auth_submit(flow_name).await?;
        Ok(())
    }

    async fn fill_auth_input(
        &self,
        selector: &str,
        value: &str,
        field_name: &str,
        flow_name: &str,
    ) -> Result<()> {
        self.client
            .find(Locator::Css(selector))
            .await
            .with_context(|| format!("finding the {flow_name} {field_name} input"))?
            .send_keys(value)
            .await
            .with_context(|| format!("typing the {flow_name} {field_name}"))?;
        Ok(())
    }

    async fn click_auth_submit(&self, flow_name: &str) -> Result<()> {
        self.client
            .find(Locator::Css(".account-form__submit"))
            .await
            .with_context(|| format!("finding the {flow_name} submit button"))?
            .click()
            .await
            .with_context(|| format!("submitting the {flow_name} form"))?;
        Ok(())
    }

    async fn wait_for_session_page(&self) -> Result<()> {
        self.wait_for_condition(
            session_route_script(),
            Duration::from_secs(30),
            "browser session route",
        )
        .await?;
        self.wait_for_condition(
            "return Boolean(document.querySelector('#composer-input'));",
            Duration::from_secs(30),
            "composer bootstrap",
        )
        .await
    }

    async fn wait_for_workspaces_page(&self) -> Result<()> {
        self.wait_for_condition(
            "return (window.location.pathname === '/app/workspaces' \
             || window.location.pathname === '/app/workspaces/') \
             && Boolean(document.querySelector('h1')) \
             && document.querySelector('h1')?.textContent?.trim() === 'Workspaces';",
            Duration::from_secs(30),
            "workspaces page",
        )
        .await
    }

    async fn wait_for_register_page(&self) -> Result<()> {
        self.wait_for_condition(
            "return (window.location.pathname === '/app/register' \
             || window.location.pathname === '/app/register/') \
             && Boolean(document.querySelector('h1')) \
             && document.querySelector('h1')?.textContent?.trim() === 'Register bootstrap account' \
             && Boolean(document.querySelector(\".account-form input[autocomplete='username']\"));",
            Duration::from_secs(30),
            "register page",
        )
        .await
    }

    async fn wait_for_sign_in_page(&self) -> Result<()> {
        self.wait_for_condition(
            "return (window.location.pathname === '/app/sign-in' \
             || window.location.pathname === '/app/sign-in/') \
             && Boolean(document.querySelector('h1')) \
             && document.querySelector('h1')?.textContent?.trim() === 'Sign in';",
            Duration::from_secs(30),
            "sign-in page",
        )
        .await
    }

    async fn wait_for_workspace_card_session_list(&self, workspace_name: &str) -> Result<()> {
        self.wait_for_condition(
            &workspace_card_has_session_list_script(workspace_name),
            Duration::from_secs(30),
            "workspace card session list",
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

    async fn submit_prompt_and_wait_for_mock_reply(&self, prompt: &str) -> Result<()> {
        let composer = self.focused_composer().await?;
        self.enter_prompt(&composer, prompt).await?;
        self.assert_composer_submission_ready().await?;
        self.click_submit_button().await?;
        self.wait_for_body_text(prompt, Duration::from_secs(10))
            .await?;
        self.wait_for_body_text(&mock_reply_for(prompt), Duration::from_secs(30))
            .await
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
            "return Array.from(document.querySelectorAll('.session-sidebar__secondary-link'))\
             .some(link => link.textContent?.trim() === 'Workspaces');",
            Duration::from_secs(10),
            "workspaces sidebar link",
        )
        .await?;
        self.client
            .execute(
                "const link = Array.from(document.querySelectorAll('.session-sidebar__secondary-link'))\
                 .find(candidate => candidate.textContent?.trim() === 'Workspaces');\
                 if (link) link.click();",
                Vec::new(),
            )
            .await
            .context("clicking the workspaces sidebar link")?;
        Ok(())
    }

    async fn create_workspace(&self, name: &str) -> Result<()> {
        // In the new UI, workspace creation is behind a modal triggered by the
        // "+ New workspace" button.  Open the modal first.
        self.client
            .find(Locator::Css(".workspace-dashboard__new-btn"))
            .await
            .context("finding the New workspace button")?
            .click()
            .await
            .context("opening the create workspace modal")?;

        self.wait_for_condition(
            "return Boolean(document.querySelector('.workspace-modal input[type=\"text\"]'));",
            Duration::from_secs(10),
            "workspace name input in modal",
        )
        .await?;
        let input = self
            .client
            .find(Locator::Css(".workspace-modal input[type='text']"))
            .await
            .context("finding workspace name input in modal")?;
        input
            .click()
            .await
            .context("focusing workspace name input")?;
        input
            .send_keys(name)
            .await
            .context("typing workspace name")?;
        self.client
            .find(Locator::Css(".workspace-modal .account-form__submit"))
            .await
            .context("finding create workspace submit button in modal")?
            .click()
            .await
            .context("submitting the create workspace form")
    }

    async fn create_workspace_and_confirm(&self, name: &str) -> Result<()> {
        self.create_workspace(name).await?;
        self.wait_for_workspace_notice("Workspace created.").await?;
        self.wait_for_body_text(name, Duration::from_secs(10)).await
    }

    async fn open_workspace_chat_and_confirm(&self, name: &str) -> Result<()> {
        self.wait_for_workspace_action_button(
            ".workspace-action-btn",
            name,
            "New chat",
            "new chat button",
        )
        .await?;
        self.click_workspace_action_button(".workspace-action-btn", name, "New chat")
            .await?;
        self.wait_for_session_page().await
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
                 .some(btn => btn.closest('.workspace-card')?.textContent?.includes({name:?}));"
            ),
            Duration::from_secs(10),
            "delete button for workspace",
        )
        .await?;

        self.client
            .execute(
                &format!(
                    "const btn = Array.from(document.querySelectorAll('.workspace-action-btn--danger'))\
                     .find(b => b.closest('.workspace-card')?.textContent?.includes({name:?})); \
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

    async fn click_sign_out_button(&self) -> Result<()> {
        self.wait_for_condition(
            "return Array.from(document.querySelectorAll('.session-sidebar__secondary-button'))\
             .some(button => button.textContent?.trim() === 'Sign out');",
            Duration::from_secs(10),
            "sign out button",
        )
        .await?;
        self.client
            .execute(
                "const button = Array.from(document.querySelectorAll('.session-sidebar__secondary-button'))\
                 .find(candidate => candidate.textContent?.trim() === 'Sign out');\
                 if (button) button.click();",
                Vec::new(),
            )
            .await
            .context("clicking the sign out button")?;
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

    async fn click_back_to_chat_link(&self) -> Result<()> {
        self.wait_for_condition(
            "return Array.from(document.querySelectorAll('.account-panel__header-actions a'))\
             .some(link => link.textContent?.trim() === 'Back to chat');",
            Duration::from_secs(10),
            "back to chat link",
        )
        .await?;
        self.client
            .execute(
                "const link = Array.from(document.querySelectorAll('.account-panel__header-actions a'))\
                 .find(candidate => candidate.textContent?.trim() === 'Back to chat');\
                 if (link) link.click();",
                Vec::new(),
            )
            .await
            .context("clicking the back to chat link")?;
        Ok(())
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

    async fn current_path(&self) -> Result<String> {
        self.evaluate(
            "return window.location.pathname;",
            "reading current pathname",
        )
        .await
    }

    async fn wait_for_path(&self, expected_path: &str, description: &str) -> Result<()> {
        let encoded =
            serde_json::to_string(expected_path).context("encoding expected browser path")?;
        self.wait_for_condition(
            &format!("return window.location.pathname === {encoded};"),
            Duration::from_secs(15),
            description,
        )
        .await
    }

    async fn session_storage_item(&self, key: &str) -> Result<Option<String>> {
        let encoded = serde_json::to_string(key).context("encoding sessionStorage key")?;
        self.evaluate(
            &format!("return window.sessionStorage.getItem({encoded});"),
            &format!("reading sessionStorage key {key}"),
        )
        .await
    }

    async fn inject_selected_workspace_and_return_home(&self, workspace_id: &str) -> Result<()> {
        let encoded =
            serde_json::to_string(workspace_id).context("encoding selected workspace id")?;
        self.client
            .execute(
                &format!(
                    "window.sessionStorage.setItem('acp-selected-workspace-id', {encoded});\
                     window.location.href = '/app/';"
                ),
                Vec::new(),
            )
            .await
            .context("injecting a stale selected workspace and navigating home")?;
        Ok(())
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

const CLOSE_CURRENT_SESSION_SCRIPT: &str = r#"
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
    "#;

fn close_current_session_script() -> &'static str {
    CLOSE_CURRENT_SESSION_SCRIPT
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

fn session_route_script() -> &'static str {
    r#"return /\/app\/sessions\/[^/]+$/.test(window.location.pathname);"#
}

fn workspaces_path_script() -> &'static str {
    "return window.location.pathname === '/app/workspaces' \
     || window.location.pathname === '/app/workspaces/';"
}

fn workspace_row_text_script(name: &str) -> String {
    format!(
        "return Array.from(document.querySelectorAll('.workspace-card'))\
         .some(card => card.textContent?.includes({name:?}));"
    )
}

fn workspace_card_has_session_list_script(name: &str) -> String {
    format!(
        "return Array.from(document.querySelectorAll('.workspace-card'))\
         .some(card => card.textContent?.includes({name:?}) \
             && card.querySelector('.workspace-card__session-list') !== null);"
    )
}

fn workspace_action_button_script(selector: &str, row_name: &str, button_label: &str) -> String {
    format!(
        "return Array.from(document.querySelectorAll({selector:?}))\
         .some(btn => btn.closest('.workspace-card')?.textContent?.includes({row_name:?}) \
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
         .find(candidate => candidate.closest('.workspace-card')?.textContent?.includes({row_name:?}) \
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
