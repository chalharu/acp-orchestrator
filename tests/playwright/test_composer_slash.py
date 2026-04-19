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
SIDEBAR_SELECTOR = ".session-sidebar"
MOCK_REPLY_TEXT = "mock assistant: I received test."
MOBILE_VIEWPORT = {"width": 390, "height": 844}


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

    def open_app(self, viewport: dict[str, int] | None = None) -> Page:
        page = self.browser.new_page(
            ignore_https_errors=True,
            viewport=viewport,
        )
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

    def test_tab_keeps_slash_text_and_moves_focus(self) -> None:
        page = self.open_app()
        composer = page.locator(TEXTAREA_SELECTOR)

        composer.click()
        page.keyboard.press("Slash")
        page.locator(PALETTE_SELECTOR).wait_for(state="visible", timeout=10_000)

        composer.press("Tab")
        page.wait_for_function(
            "() => document.activeElement?.classList.contains('composer__submit')",
            timeout=10_000,
        )

        self.assertEqual(composer.input_value(), "/")

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

    def test_pending_reply_does_not_steal_focus_back_after_sidebar_click(self) -> None:
        page = self.open_app()
        composer = page.locator(TEXTAREA_SELECTOR)
        submit = page.locator(SUBMIT_SELECTOR)
        toggle = page.locator(".session-sidebar__toggle")

        composer.click()
        page.keyboard.type("test", delay=100)
        submit.click()

        page.wait_for_function(
            "() => document.querySelector('#composer-input')?.disabled === true",
            timeout=10_000,
        )
        toggle.click()
        page.wait_for_function(
            "() => document.activeElement?.classList.contains('session-sidebar__toggle')",
            timeout=10_000,
        )

        page.get_by_text(MOCK_REPLY_TEXT).wait_for(timeout=30_000)
        page.wait_for_timeout(500)

        self.assertNotEqual(
            page.evaluate("() => document.activeElement?.id"), "composer-input"
        )
        self.assertTrue(
            page.evaluate(
                "() => document.activeElement?.classList.contains('session-sidebar__toggle')"
            )
        )

    def test_pending_reply_does_not_steal_focus_back_after_keyboard_focus_change(
        self,
    ) -> None:
        page = self.open_app()
        composer = page.locator(TEXTAREA_SELECTOR)
        submit = page.locator(SUBMIT_SELECTOR)
        toggle = page.locator(".session-sidebar__toggle")

        composer.click()
        page.keyboard.type("test", delay=100)
        submit.click()

        page.wait_for_function(
            "() => document.querySelector('#composer-input')?.disabled === true",
            timeout=10_000,
        )
        toggle.focus()
        page.wait_for_function(
            "() => document.activeElement?.classList.contains('session-sidebar__toggle')",
            timeout=10_000,
        )

        page.get_by_text(MOCK_REPLY_TEXT).wait_for(timeout=30_000)
        page.wait_for_timeout(500)

        self.assertNotEqual(
            page.evaluate("() => document.activeElement?.id"), "composer-input"
        )
        self.assertTrue(
            page.evaluate(
                "() => document.activeElement?.classList.contains('session-sidebar__toggle')"
            )
        )

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

    def test_clicking_outside_session_rename_input_commits_the_title(self) -> None:
        page = self.open_app()
        composer = page.locator(TEXTAREA_SELECTOR)
        rename_button = page.get_by_role("button", name="Rename session").first
        rename_input = page.locator(".session-sidebar__rename-input")
        title = page.locator(".session-sidebar__session-title").first
        renamed_title = "Blur committed title"

        rename_button.click()
        rename_input.wait_for(state="visible", timeout=10_000)
        rename_input.fill(renamed_title)

        composer.click()
        page.wait_for_function(
            """([selector, expected]) => {
                const node = document.querySelector(selector);
                return node !== null && node.textContent.trim() === expected;
            }""",
            arg=[".session-sidebar__session-title", renamed_title],
            timeout=10_000,
        )

        self.assertFalse(rename_input.is_visible())
        self.assertEqual(title.text_content().strip(), renamed_title)

    def test_mobile_sidebar_keeps_actions_visible_and_dismisses_from_corner_taps(
        self,
    ) -> None:
        page = self.open_app(viewport=MOBILE_VIEWPORT)
        sidebar = page.locator(SIDEBAR_SELECTOR)
        toggle = page.locator(".session-sidebar__toggle")

        toggle.click()
        sidebar.wait_for(state="visible", timeout=10_000)

        new_chat = page.locator(".session-sidebar__new-link")
        new_chat_label = page.locator(".session-sidebar__new-link-label")
        dismiss = page.get_by_role("button", name="Close sidebar")
        title = page.locator(".session-sidebar__session-title").first
        delete_button = page.get_by_role("button", name="Delete session").first

        new_chat_box = new_chat.bounding_box()
        dismiss_box = dismiss.bounding_box()
        title_box = title.bounding_box()

        self.assertEqual(
            new_chat_label.evaluate("node => getComputedStyle(node).display"),
            "none",
        )
        self.assertIsNotNone(new_chat_box)
        self.assertIsNotNone(dismiss_box)
        self.assertIsNotNone(title_box)
        self.assertLess(abs(new_chat_box["width"] - dismiss_box["width"]), 8)
        self.assertGreater(title_box["width"], 48)
        self.assertTrue(delete_button.is_visible())
        self.assertEqual(
            sidebar.evaluate("node => getComputedStyle(node).boxShadow"),
            "none",
        )

        page.mouse.click(MOBILE_VIEWPORT["width"] - 12, 12)
        page.wait_for_function(
            "() => getComputedStyle(document.querySelector('.session-sidebar')).display === 'none'",
            timeout=10_000,
        )
        self.assertEqual(toggle.get_attribute("aria-expanded"), "false")


if __name__ == "__main__":
    unittest.main()
