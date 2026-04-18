"""Browser regression coverage for the Web composer slash flow.

Run against a live web stack with:

    python3 -m unittest discover -s tests/playwright -p 'test_*.py'

Set ACP_PLAYWRIGHT_SYSROOT when Chromium needs a user-space library/font sysroot.
Set ACP_WEB_APP_URL to point at a different app URL.
"""

from __future__ import annotations

import os
from pathlib import Path
import re
import unittest

from playwright.sync_api import Page, sync_playwright


APP_URL = os.environ.get("ACP_WEB_APP_URL", "https://127.0.0.1:18080/app/")
PLAYWRIGHT_SYSROOT_ENV = "ACP_PLAYWRIGHT_SYSROOT"
TEXTAREA_SELECTOR = "#composer-input"
PALETTE_SELECTOR = ".composer__slash-palette"
PALETTE_ITEM_SELECTOR = ".composer__slash-item"
SUBMIT_SELECTOR = ".composer__submit"
MOCK_REPLY_TEXT = "mock assistant: I received test."


def chromium_env() -> dict[str, str]:
    env = os.environ.copy()
    sysroot = os.environ.get(PLAYWRIGHT_SYSROOT_ENV)
    if not sysroot:
        return env

    root = Path(sysroot)
    lib_dirs = sorted({str(path.parent) for path in root.rglob("lib*.so*")})
    if lib_dirs:
        current = env.get("LD_LIBRARY_PATH")
        env["LD_LIBRARY_PATH"] = ":".join([*lib_dirs, *([current] if current else [])])

    fonts_conf = root / "etc" / "fonts" / "fonts.conf"
    if fonts_conf.exists():
        env["FONTCONFIG_PATH"] = str(fonts_conf.parent)
        env["FONTCONFIG_FILE"] = str(fonts_conf)
        env["FONTCONFIG_SYSROOT"] = str(root)

    return env


class ComposerSlashPlaywrightTest(unittest.TestCase):
    def setUp(self) -> None:
        self.playwright = None
        self.browser = None
        try:
            self.playwright = sync_playwright().start()
            self.browser = self.playwright.chromium.launch(
                headless=True,
                args=["--no-sandbox", "--disable-gpu"],
                env=chromium_env(),
            )
        except Exception:
            if self.playwright is not None:
                self.playwright.stop()
            raise

    def tearDown(self) -> None:
        if self.browser is not None:
            self.browser.close()
        if self.playwright is not None:
            self.playwright.stop()

    def open_app(self) -> Page:
        page = self.browser.new_page(ignore_https_errors=True)
        page.goto(APP_URL, wait_until="domcontentloaded", timeout=30_000)
        page.locator(TEXTAREA_SELECTOR).wait_for(state="visible", timeout=30_000)
        page.wait_for_url(re.compile(r".*/app/sessions/[^/]+$"), timeout=30_000)
        page.wait_for_timeout(1_500)
        return page

    def test_slash_palette_opens_and_single_click_applies(self) -> None:
        page = self.open_app()
        composer = page.locator(TEXTAREA_SELECTOR)

        composer.click()
        page.keyboard.press("Slash")

        palette = page.locator(PALETTE_SELECTOR)
        palette.wait_for(state="visible", timeout=10_000)
        items = page.locator(PALETTE_ITEM_SELECTOR)
        self.assertGreater(items.count(), 0)
        item_texts = [text.strip() for text in items.all_inner_texts()]

        self.assertTrue(any("/help" in text for text in item_texts))
        self.assertFalse(any("/cancel" in text for text in item_texts))
        self.assertFalse(any("/approve" in text for text in item_texts))
        self.assertFalse(any("/deny" in text for text in item_texts))
        self.assertFalse(any("/quit" in text for text in item_texts))

        items.first.click()
        page.wait_for_timeout(500)

        applied_value = composer.input_value()
        self.assertNotEqual(applied_value, "/")
        self.assertTrue(applied_value.startswith("/"))

    def test_clearing_slash_command_keeps_send_enabled_and_submits_text(self) -> None:
        page = self.open_app()
        composer = page.locator(TEXTAREA_SELECTOR)
        submit = page.locator(SUBMIT_SELECTOR)

        composer.click()
        page.keyboard.type("/help", delay=100)
        page.wait_for_timeout(800)
        composer.press("Control+A")
        composer.press("Backspace")
        page.keyboard.type("test", delay=100)
        page.wait_for_timeout(500)

        self.assertFalse(composer.is_disabled())
        self.assertFalse(submit.is_disabled())

        submit.click()
        page.get_by_text(MOCK_REPLY_TEXT).wait_for(timeout=30_000)

        self.assertEqual(composer.input_value(), "")

    def test_sending_restores_focus_to_the_composer(self) -> None:
        page = self.open_app()
        composer = page.locator(TEXTAREA_SELECTOR)
        submit = page.locator(SUBMIT_SELECTOR)

        composer.click()
        page.keyboard.type("test", delay=100)
        submit.click()

        page.get_by_text(MOCK_REPLY_TEXT).wait_for(timeout=30_000)
        page.wait_for_function(
            "() => document.activeElement?.id === 'composer-input'",
            timeout=10_000,
        )

        self.assertEqual(composer.input_value(), "")

    def test_deleting_only_session_opens_a_fresh_chat(self) -> None:
        page = self.open_app()
        composer = page.locator(TEXTAREA_SELECTOR)
        initial_url = page.url

        page.get_by_role("button", name="Delete session").first.click()
        page.wait_for_function(
            "(previousUrl) => window.location.href !== previousUrl",
            arg=initial_url,
            timeout=30_000,
        )
        page.wait_for_url(re.compile(r".*/app/sessions/[^/]+$"), timeout=30_000)
        composer.wait_for(state="visible", timeout=30_000)

        self.assertNotEqual(page.url, initial_url)
        self.assertFalse(
            page.get_by_text("Session unavailable. Start a fresh chat.").is_visible()
        )
        self.assertFalse(composer.is_disabled())


if __name__ == "__main__":
    unittest.main()
