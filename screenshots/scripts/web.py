"""Screenshots of the web UI of MyComputer, in the color scheme of argv[1]: its pages, or with argv[2] = runs, the
recent runs of a backup of every collection."""

import re
import sys

from playwright.sync_api import FloatRect, Page, sync_playwright

URL, TOKEN = "http://mycomputer:32110", "screenshots"
DOCUMENTS = f"{URL}/collections/Documents-hdd"
SCHEME = sys.argv[1]


def shot(page: Page, name: str, full_page: bool = True, clip: FloatRect | None = None) -> None:
    page.wait_for_load_state()
    page.wait_for_timeout(500)
    # A page shorter than the window ends below its footer, with the margin above its top bar.
    if full_page and clip is None:
        # Measured from the top of the page, which a click may have scrolled.
        page.evaluate("window.scrollTo(0, 0)")
        top, footer = page.locator("header.topbar").bounding_box(), page.locator(".page > footer").bounding_box()
        bottom = min(footer["y"] + footer["height"] + top["y"], page.evaluate("document.documentElement.scrollHeight"))
        clip = {"x": 0, "y": 0, "width": 1200, "height": bottom}
    page.screenshot(path=f"/screenshots/web-{SCHEME}-{name}.png", full_page=full_page, clip=clip)


def dialog(page: Page, title: str, name: str) -> None:
    page.goto(DOCUMENTS)
    page.get_by_title(title, exact=True).click()
    page.locator("#dialog form").wait_for()
    shot(page, name, full_page=False)


def pages(page: Page) -> None:
    page.goto(URL)
    # The README's first picture: the collections, without the recent runs below them.
    runs = page.locator("#recent-runs").bounding_box()
    shot(page, "collections", full_page=False, clip={"x": 0, "y": 0, "width": 1200, "height": runs["y"] - 8})
    page.goto(DOCUMENTS)
    shot(page, "collection")
    dialog(page, "Restore", "restore")

    page.goto(f"{DOCUMENTS}/repos/Documents-hdd")
    shot(page, "snapshots")
    # The newest snapshot but one: a scan changed, one deleted and one added.
    page.locator("a[href*='/snapshots/']:not([href$=download])").nth(1).click()
    page.get_by_role("link", name="Invoices").click()
    shot(page, "browse")
    page.get_by_role("link", name="Changes").click()
    shot(page, "changes")

    page.goto(f"{URL}/setup")
    # Their labels cover the inputs.
    page.locator("label", has=page.locator("input[name=target][value=gdrive]")).click()
    page.locator(".choices label", has_text=re.compile("ntfy", re.IGNORECASE)).click()
    shot(page, "setup")


def runs(page: Page) -> None:
    page.goto(URL)
    # The two backups running, and the two runs before them, without the notice the button shows.
    page.add_style_tag(content="#flash { display: none } #recent-runs tbody tr:nth-child(n+5) { display: none }")
    page.get_by_role("button", name="Back up all").click()
    # Pictures waits for the disk and CPU that the snapshot of Documents holds: a tag in "Waiting for".
    page.locator("#recent-runs td:nth-child(8) .tag").first.wait_for(timeout=30_000)
    page.wait_for_timeout(300)
    page.locator("#recent-runs").screenshot(path=f"/screenshots/web-{SCHEME}-runs.png")
    page.locator("#recent-runs .spinner").first.wait_for(state="detached", timeout=300_000)


with sync_playwright() as p:
    browser = p.chromium.launch()
    # The runs card hides rows with a style, which the page's policy would refuse.
    viewport = {"width": 1200, "height": 800}
    context = browser.new_context(viewport=viewport, device_scale_factor=2, color_scheme=SCHEME, bypass_csp=True)
    context.request.post(f"{URL}/login", form={"token": TOKEN})
    page = context.new_page()
    if sys.argv[2:] == ["runs"]:
        runs(page)
    else:
        pages(page)
    browser.close()
