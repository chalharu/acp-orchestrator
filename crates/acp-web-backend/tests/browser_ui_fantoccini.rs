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

use support::{
    ServerConfig, TestStack, build_browser_client, extract_meta_content, load_browser_app_shell,
    register_browser_account, test_state_dir,
};

const APP_PATH: &str = "/app/";
const BROWSER_TEST_USER_NAME: &str = "browser-test";
const BROWSER_TEST_PASSWORD: &str = "browser-test-password";
const SIGN_IN_INPUT_SELECTOR: &str = "#sign-in-user-name";
const SIGN_IN_PASSWORD_SELECTOR: &str = "#sign-in-password";
const SIGN_IN_BUTTON_SELECTOR: &str = ".auth-form__submit";
const COMPOSER_SELECTOR: &str = "#composer-input";
const SUBMIT_SELECTOR: &str = ".composer__submit";
const SIDEBAR_TOGGLE_SELECTOR: &str = ".session-sidebar__toggle";
const MOCK_REPLY_TEXT: &str = "mock assistant: I received test.";
const CLOSED_SESSION_TEXT: &str = "This conversation has ended.";
const WEBDRIVER_READY_ATTEMPTS: usize = 50;
const WEBDRIVER_READY_DELAY: Duration = Duration::from_millis(100);
const WEBDRIVER_START_RETRIES: usize = 5;
const SLASH_PALETTE_READY_SCRIPT: &str = "return Boolean(document.querySelector('.composer__slash-palette')) \
     && document.querySelectorAll('.composer__slash-item').length > 0;";
const SLASH_ITEM_TEXTS_SCRIPT: &str = "return Array.from(document.querySelectorAll('.composer__slash-item'))\
    .map((item) => item.textContent.trim());";
const COMPOSER_EMPTY_SCRIPT: &str =
    "return document.querySelector('#composer-input')?.value === '';";
const COMPOSER_DISABLED_SCRIPT: &str =
    "return document.querySelector('#composer-input')?.disabled ?? true;";
const SUBMIT_DISABLED_SCRIPT: &str =
    "return document.querySelector('.composer__submit')?.disabled ?? true;";
const AUTH_BOOTSTRAP_READY_SCRIPT: &str = "return Boolean(document.querySelector('#sign-in-user-name')) || Boolean(document.querySelector('#composer-input'));";
const SESSION_ROUTE_SCRIPT: &str =
    r#"return /\/app\/sessions\/[^/]+$/.test(window.location.pathname);"#;
const SIDEBAR_VISIBLE_SCRIPT: &str = "const node = document.querySelector('.session-sidebar'); \
    return Boolean(node) && getComputedStyle(node).display !== 'none';";
const SIDEBAR_METADATA_READY_SCRIPT: &str = "return Boolean(document.querySelector('.session-sidebar__session-activity')) \
     && Boolean(document.querySelector('.session-sidebar__status-pill'));";
const SESSION_ACTIVITY_LABEL_SCRIPT: &str = "return document.querySelector('.session-sidebar__session-activity')\
    ?.textContent?.trim() ?? '';";
const SESSION_STATUS_LABEL_SCRIPT: &str = "return document.querySelector('.session-sidebar__status-pill')\
    ?.textContent?.trim() ?? '';";
const SESSION_CLOSED_SCRIPT: &str = "return document.querySelector('.session-sidebar__status-pill')\
    ?.textContent?.trim() === 'closed';";
const CLOSE_SESSION_SCRIPT: &str = r#"
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

#[tokio::test]
#[ignore = "requires ChromeDriver, Chrome, and a built frontend bundle"]
async fn slash_prefix_can_be_removed_without_breaking_prompt_submission() -> Result<()> {
    let browser = BrowserHarness::spawn((1280, 960)).await?;
    let result = async {
        browser.open_app().await?;
        browser.focus_composer().await?;
        browser.type_in_composer("/").await?;
        browser.wait_for_slash_palette().await?;
        assert_slash_palette_contents(&browser.slash_item_texts().await?);
        browser.delete_composer_prefix().await?;
        browser.wait_for_empty_composer().await?;
        browser.type_in_composer("test").await?;
        assert_prompt_ready(&browser).await?;
        browser.click_submit().await?;
        browser
            .wait_for_body_text(MOCK_REPLY_TEXT, Duration::from_secs(30))
            .await?;
        assert_eq!(browser.composer_value().await?, "");

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
        assert_sidebar_metadata(&browser).await?;
        browser.close_session().await?;
        browser.wait_for_closed_status().await?;
        browser
            .wait_for_body_text(CLOSED_SESSION_TEXT, Duration::from_secs(10))
            .await?;

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
        provision_browser_account(&stack.backend_url).await?;

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

        self.wait_for_condition(
            AUTH_BOOTSTRAP_READY_SCRIPT,
            Duration::from_secs(30),
            "auth bootstrap",
        )
        .await?;
        if self.sign_in_required().await? {
            self.sign_in_as(BROWSER_TEST_USER_NAME, BROWSER_TEST_PASSWORD)
                .await?;
        }
        self.wait_for_condition(
            "return Boolean(document.querySelector('#composer-input'));",
            Duration::from_secs(30),
            "composer bootstrap",
        )
        .await?;
        self.wait_for_condition(
            SESSION_ROUTE_SCRIPT,
            Duration::from_secs(30),
            "browser session route",
        )
        .await
    }

    async fn sign_in_required(&self) -> Result<bool> {
        self.evaluate(
            "return Boolean(document.querySelector('#sign-in-user-name'));",
            "checking sign-in state",
        )
        .await
    }

    async fn sign_in_as(&self, user_name: &str, password: &str) -> Result<()> {
        self.client
            .find(Locator::Css(SIGN_IN_INPUT_SELECTOR))
            .await
            .context("finding the sign-in user-name input")?
            .send_keys(user_name)
            .await
            .with_context(|| format!("typing {user_name:?} into the sign-in form"))?;
        self.client
            .find(Locator::Css(SIGN_IN_PASSWORD_SELECTOR))
            .await
            .context("finding the sign-in password input")?
            .send_keys(password)
            .await
            .context("typing the sign-in password")?;
        self.client
            .find(Locator::Css(SIGN_IN_BUTTON_SELECTOR))
            .await
            .context("finding the sign-in submit button")?
            .click()
            .await
            .context("submitting the sign-in form")
    }

    async fn ensure_sidebar_visible(&self) -> Result<()> {
        let is_visible: bool = self
            .evaluate(SIDEBAR_VISIBLE_SCRIPT, "checking sidebar visibility")
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
            SIDEBAR_VISIBLE_SCRIPT,
            Duration::from_secs(10),
            "visible session sidebar",
        )
        .await
    }

    async fn focus_composer(&self) -> Result<()> {
        self.client
            .find(Locator::Css(COMPOSER_SELECTOR))
            .await
            .context("finding the composer textarea")?
            .click()
            .await
            .context("focusing the composer")
    }

    async fn type_in_composer(&self, text: &str) -> Result<()> {
        self.client
            .find(Locator::Css(COMPOSER_SELECTOR))
            .await
            .context("finding the composer textarea")?
            .send_keys(text)
            .await
            .with_context(|| format!("typing {text:?} into the composer"))
    }

    async fn delete_composer_prefix(&self) -> Result<()> {
        self.client
            .find(Locator::Css(COMPOSER_SELECTOR))
            .await
            .context("finding the composer textarea")?
            .send_keys(&Key::Backspace.to_string())
            .await
            .context("deleting the slash prefix")
    }

    async fn wait_for_slash_palette(&self) -> Result<()> {
        self.wait_for_condition(
            SLASH_PALETTE_READY_SCRIPT,
            Duration::from_secs(10),
            "slash command palette",
        )
        .await
    }

    async fn slash_item_texts(&self) -> Result<Vec<String>> {
        self.evaluate(SLASH_ITEM_TEXTS_SCRIPT, "reading slash command labels")
            .await
    }

    async fn wait_for_empty_composer(&self) -> Result<()> {
        self.wait_for_condition(
            COMPOSER_EMPTY_SCRIPT,
            Duration::from_secs(10),
            "empty composer after removing slash",
        )
        .await
    }

    async fn composer_disabled(&self) -> Result<bool> {
        self.evaluate(COMPOSER_DISABLED_SCRIPT, "checking composer enabled state")
            .await
    }

    async fn submit_disabled(&self) -> Result<bool> {
        self.evaluate(SUBMIT_DISABLED_SCRIPT, "checking submit enabled state")
            .await
    }

    async fn click_submit(&self) -> Result<()> {
        self.client
            .find(Locator::Css(SUBMIT_SELECTOR))
            .await
            .context("finding the submit button")?
            .click()
            .await
            .context("submitting the prompt")
    }

    async fn composer_value(&self) -> Result<String> {
        self.evaluate(
            "return document.querySelector('#composer-input')?.value ?? '';",
            "reading composer value after submit",
        )
        .await
    }

    async fn wait_for_sidebar_metadata(&self) -> Result<()> {
        self.wait_for_condition(
            SIDEBAR_METADATA_READY_SCRIPT,
            Duration::from_secs(10),
            "sidebar metadata",
        )
        .await
    }

    async fn session_activity_label(&self) -> Result<String> {
        self.evaluate(
            SESSION_ACTIVITY_LABEL_SCRIPT,
            "reading session activity label",
        )
        .await
    }

    async fn session_status_label(&self) -> Result<String> {
        self.evaluate(SESSION_STATUS_LABEL_SCRIPT, "reading session status label")
            .await
    }

    async fn close_session(&self) -> Result<()> {
        let close_result = self
            .client
            .execute_async(CLOSE_SESSION_SCRIPT, Vec::new())
            .await
            .context("closing the session from the browser")?;
        ensure_close_result_ok(close_result)
    }

    async fn wait_for_closed_status(&self) -> Result<()> {
        self.wait_for_condition(
            SESSION_CLOSED_SCRIPT,
            Duration::from_secs(10),
            "closed status pill",
        )
        .await
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

async fn provision_browser_account(backend_url: &str) -> Result<()> {
    let client = build_browser_client()?;
    let app_document = load_browser_app_shell(&client, backend_url).await?;
    let csrf_token = extract_meta_content(&app_document, "acp-csrf-token")?;
    let response = register_browser_account(
        &client,
        backend_url,
        &csrf_token,
        BROWSER_TEST_USER_NAME,
        BROWSER_TEST_PASSWORD,
    )
    .await?;
    ensure!(
        response.authenticated,
        "browser test account should be authenticated"
    );
    Ok(())
}

fn assert_slash_palette_contents(item_texts: &[String]) {
    assert!(item_texts.iter().any(|text| text.contains("/help")));
    assert!(!item_texts.iter().any(|text| text.contains("/cancel")));
    assert!(!item_texts.iter().any(|text| text.contains("/approve")));
    assert!(!item_texts.iter().any(|text| text.contains("/deny")));
    assert!(!item_texts.iter().any(|text| text.contains("/quit")));
}

async fn assert_prompt_ready(browser: &BrowserHarness) -> Result<()> {
    assert!(!browser.composer_disabled().await?);
    assert!(!browser.submit_disabled().await?);
    Ok(())
}

async fn assert_sidebar_metadata(browser: &BrowserHarness) -> Result<()> {
    browser.wait_for_sidebar_metadata().await?;
    let activity_label = browser.session_activity_label().await?;
    assert_eq!(browser.session_status_label().await?, "active");
    assert_activity_label_shape(&activity_label)
}

fn assert_activity_label_shape(activity_label: &str) -> Result<()> {
    assert!(activity_label.starts_with("Updated "));
    assert!(activity_label.ends_with(" UTC"));
    let timestamp = activity_label
        .strip_prefix("Updated ")
        .and_then(|value| value.strip_suffix(" UTC"))
        .context("sidebar activity label did not match the expected shape")?;
    NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d %H:%M")
        .context("parsing the sidebar activity timestamp")?;
    Ok(())
}

fn ensure_close_result_ok(close_result: Value) -> Result<()> {
    let close_payload = close_result
        .as_object()
        .context("close response was not an object")?;
    ensure!(
        close_payload.get("ok").and_then(Value::as_bool) == Some(true),
        "browser close request failed: {close_result}"
    );
    Ok(())
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
