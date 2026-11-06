"""Headless Chrome smoke test for the opt-in WebGL2 GUI preview.

Run after `cargo run --locked --example city_gui_preview`:
    python scripts/city_preview_browser_smoke.py

Requires locally installed Chrome/Chromium and the Python websocket-client
package. This is an explicit interactive-rendering check, not a CI dependency.
It creates and cleans up its own isolated headless Chrome profile.
"""

from __future__ import annotations

import base64
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time
from urllib.request import urlopen

try:
    import websocket
except ImportError as error:
    raise SystemExit("websocket-client is required for the optional browser smoke test") from error


PROJECT = Path(__file__).resolve().parent.parent
PREVIEW = PROJECT / "target" / "agent-output" / "city_preview"
PROFILE = PREVIEW / "smoke-chrome-profile"
SCREENSHOT = PREVIEW / "smoke-selected.png"
NARROW_SCREENSHOT = PREVIEW / "smoke-narrow.png"
OVERVIEW_SCREENSHOT = PREVIEW / "smoke-overview.png"


def find_browser() -> str:
    candidates = [
        os.environ.get("CRIMOCRACY_CHROME", ""),
        shutil.which("chrome"),
        shutil.which("chromium"),
        shutil.which("msedge"),
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
    ]
    for candidate in candidates:
        if candidate and Path(candidate).is_file():
            return str(candidate)
    raise RuntimeError("Chrome, Chromium, or Edge executable not found")


class DevTools:
    def __init__(self, connection):
        self.connection = connection
        self.next_id = 0

    def call(self, method: str, params: dict | None = None) -> dict:
        self.next_id += 1
        request_id = self.next_id
        self.connection.send(
            json.dumps({"id": request_id, "method": method, "params": params or {}})
        )
        while True:
            message = json.loads(self.connection.recv())
            if message.get("id") != request_id:
                continue
            if "error" in message:
                raise RuntimeError(f"{method}: {message['error']}")
            return message["result"]

    def evaluate(self, expression: str):
        result = self.call(
            "Runtime.evaluate",
            {
                "expression": expression,
                "returnByValue": True,
                "awaitPromise": True,
            },
        )
        if "exceptionDetails" in result:
            raise RuntimeError(f"JS evaluation failed: {result['exceptionDetails']}")
        return result["result"].get("value")

    def click(self, x: float, y: float) -> None:
        for event_type in ("mousePressed", "mouseReleased"):
            self.call(
                "Input.dispatchMouseEvent",
                {
                    "type": event_type,
                    "x": x,
                    "y": y,
                    "button": "left",
                    "clickCount": 1,
                },
            )

    def next_frame(self) -> None:
        self.evaluate("new Promise(resolve => requestAnimationFrame(() => resolve(true)))")


def connect(process: subprocess.Popen) -> DevTools:
    endpoint = PROFILE / "DevToolsActivePort"
    for _ in range(100):
        if endpoint.is_file():
            lines = endpoint.read_text().splitlines()
            if lines:
                port = int(lines[0])
                with urlopen(f"http://127.0.0.1:{port}/json/list", timeout=5) as response:
                    targets = json.load(response)
                pages = [target for target in targets if target.get("type") == "page"]
                if pages:
                    connection = websocket.create_connection(
                        pages[0]["webSocketDebuggerUrl"],
                        timeout=12,
                        origin="http://localhost",
                    )
                    return DevTools(connection)
        if process.poll() is not None:
            raise RuntimeError(f"Headless browser exited early: {process.returncode}")
        time.sleep(0.1)
    raise RuntimeError("Headless browser did not expose a debugging endpoint")


def check(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def verify_export_disclosure(html: str) -> None:
    check("Unlisted Private Depot" not in html, "private property names must not enter the HTML")
    match = re.search(
        r'<script id="city-preview-data" type="application/json">(.*?)</script>',
        html,
        flags=re.DOTALL,
    )
    check(match is not None, "generated HTML must embed one readable scene payload")
    scene = json.loads(match.group(1))
    disclosed = {int(entry["id"]) for entry in scene["dossiers"]}
    encoded_ids = set()
    for district in scene["districts"]:
        for batch in district["buildings"]:
            payload = base64.b64decode(batch["bytes"], validate=True)
            check(
                len(payload) == int(batch["instance_count"]) * 32,
                "each GPU instance buffer must have the documented stride",
            )
            for offset in range(24, len(payload), 32):
                pick_id = int.from_bytes(payload[offset:offset + 4], "little")
                if pick_id:
                    encoded_ids.add(pick_id)
    check(
        encoded_ids == disclosed,
        f"GPU picking data must contain exactly the public directory IDs; leaked {encoded_ids - disclosed}",
    )
    check(len(disclosed) == 96, "preview must contain 96 intentionally public entries")


def run_checks(cdp: DevTools) -> None:
    for _ in range(80):
        if cdp.evaluate("document.documentElement?.dataset.webgl ?? null") == "ready":
            break
        time.sleep(0.1)
    else:
        message = cdp.evaluate("document.querySelector('#fatal-error').textContent")
        raise AssertionError(f"WebGL2 scene did not initialize: {message}")

    statistics = cdp.evaluate("JSON.stringify(crimocracyCityPreview.stats())")
    initial = json.loads(statistics)
    check(initial["buildings"] > 0, "camera must draw buildings")
    check(initial["drawCalls"] > 0, "camera must submit draw calls")
    check(
        0 < initial["uploadedDistricts"] < 8,
        "initial frame must upload visible districts without loading the entire city",
    )
    check(cdp.evaluate("document.querySelectorAll('.ward-button').length") == 8, "eight wards expected")
    check(
        cdp.evaluate("document.querySelectorAll('.property-button').length") == 96,
        "public directory must list all 96 sample properties",
    )
    check(
        0 < cdp.evaluate("document.querySelectorAll('.world-label').length") <= 24,
        "only a bounded number of on-screen property labels should exist in the DOM",
    )
    check(
        cdp.evaluate("document.querySelectorAll('.world-ward').length") == 0,
        "ward labels should not be allocated in close-up view",
    )

    cdp.evaluate(
        """(() => {
          const input = document.querySelector('#search');
          input.value = 'motor';
          input.dispatchEvent(new Event('input', {bubbles:true}));
        })()"""
    )
    check(cdp.evaluate("document.querySelectorAll('.property-button').length") == 8, "directory search must filter by name")
    cdp.evaluate(
        """(() => {
          const input = document.querySelector('#search');
          input.value = '';
          input.dispatchEvent(new Event('input', {bubbles:true}));
        })()"""
    )

    cdp.evaluate("crimocracyCityPreview.showWard(8)")
    cdp.next_frame()
    visited_count = json.loads(cdp.evaluate("JSON.stringify(crimocracyCityPreview.stats())"))["uploadedDistricts"]
    check(visited_count > initial["uploadedDistricts"], "visiting a distant ward must upload its district on demand")

    # The GPU cache must release distant geometry and restore exact geometry
    # and picking IDs when the user revisits a previously evicted district.
    for ward_id in range(1, 9):
        cdp.evaluate(f"crimocracyCityPreview.showWard({ward_id})")
        cdp.next_frame()
        status = json.loads(cdp.evaluate("JSON.stringify(crimocracyCityPreview.stats())"))
        check(
            status["residentDistricts"]
            <= max(status["maxResidentDistricts"], status["visibleDistricts"]),
            f"GPU cache retains unnecessary offscreen districts in ward {ward_id}",
        )
        check(status["residentInstanceBytes"] > 0, "GPU instance memory must be accounted for")
    after_tour = json.loads(cdp.evaluate("JSON.stringify(crimocracyCityPreview.stats())"))
    check(after_tour["uploadedDistricts"] == 8, "a complete ward tour must visit every district")
    check(after_tour["evictedDistricts"] > 0, "off-screen GPU districts must be evicted under budget")
    uploads_before_return = after_tour["gpuUploads"]
    cdp.evaluate("crimocracyCityPreview.showWard(1)")
    cdp.next_frame()
    after_return = json.loads(cdp.evaluate("JSON.stringify(crimocracyCityPreview.stats())"))
    check(
        after_return["gpuUploads"] > uploads_before_return,
        "revisiting a previously evicted ward must re-upload its geometry",
    )
    check(after_return["uploadedDistricts"] == 8, "re-uploads must not count as newly visited wards")

    cdp.evaluate("document.querySelector('#city-overview').click()")
    cdp.next_frame()
    overview = json.loads(cdp.evaluate("JSON.stringify(crimocracyCityPreview.stats())"))
    check(overview["visibleDistricts"] == 8, "city overview should frame all eight wards")
    check(overview["buildings"] == 8192, "overview should render every building")
    check(
        overview["residentDistricts"] == 8,
        "GPU budget must not evict districts currently visible in the overview",
    )
    check(
        cdp.evaluate("document.querySelectorAll('.world-label').length") == 0,
        "off-screen property labels must be removed from the DOM",
    )
    check(
        0 < cdp.evaluate("document.querySelectorAll('.world-ward').length") <= 8,
        "the overview should instantiate only visible ward labels",
    )
    check(
        not cdp.evaluate(
            """(() => {
              const boxes = [...document.querySelectorAll('.world-ward')]
                .map(node => node.getBoundingClientRect());
              return boxes.some((a, i) => boxes.slice(i + 1).some(b =>
                a.left < b.right && a.right > b.left
                && a.top < b.bottom && a.bottom > b.top));
            })()"""
        ),
        "visible ward labels should not overlap in the actual browser layout",
    )
    overview_image = cdp.call(
        "Page.captureScreenshot", {"format": "png", "captureBeyondViewport": False}
    )
    OVERVIEW_SCREENSHOT.write_bytes(base64.b64decode(overview_image["data"]))

    cdp.evaluate("crimocracyCityPreview.showWard(3)")
    cdp.next_frame()
    after_overview = json.loads(cdp.evaluate("JSON.stringify(crimocracyCityPreview.stats())"))
    check(
        after_overview["residentDistricts"] <= max(
            after_overview["maxResidentDistricts"], after_overview["visibleDistricts"]
        ) and after_overview["residentDistricts"] < 8,
        f"returning from overview should evict only off-screen geometry: {after_overview}",
    )
    check(
        cdp.evaluate("document.querySelector('.ward-button.active').textContent.includes('Financial Quarter')"),
        "ward navigation must update active ward",
    )

    # Exercise the actual picking framebuffer, not just the directory's
    # selection callback. Find a clearly exposed roof under the camera.
    picked_id = None
    for candidate in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]:
        cdp.evaluate(
            f"crimocracyCityPreview.focusOnProperty({candidate}); crimocracyCityPreview.selectBusiness(0)"
        )
        cdp.next_frame()
        before_pick = json.loads(cdp.evaluate("JSON.stringify(crimocracyCityPreview.stats())"))
        coordinates = cdp.evaluate(
            f"JSON.stringify(crimocracyCityPreview.projectProperty({candidate}))"
        )
        x, y = json.loads(coordinates)
        cdp.click(x, y)
        after_pick = json.loads(cdp.evaluate("JSON.stringify(crimocracyCityPreview.stats())"))
        check(
            after_pick["buildings"] == before_pick["buildings"],
            "GPU picking must not erase normal-render building statistics",
        )
        selected = int(cdp.evaluate("document.documentElement.dataset.selectedBusiness"))
        if selected == candidate:
            picked_id = selected
            break
    check(picked_id is not None, "GPU picking must resolve at least one visible business roof to its persistent ID")
    check(
        cdp.evaluate("document.querySelector('#dossier-name').textContent.length") > 0,
        "GPU-picked business must open a populated dossier",
    )
    check(
        cdp.evaluate("document.querySelector('#dossier-detail').hidden") is False,
        "selected property details must be visible",
    )
    focus_visible = cdp.evaluate(
        """(() => {
            const button = document.querySelector('#focus-property');
            const panel = document.querySelector('#dossier');
            const a = button.getBoundingClientRect();
            const b = panel.getBoundingClientRect();
            return !button.hidden && a.top >= b.top && a.bottom <= b.bottom;
        })()"""
    )
    check(focus_visible, "focus action must remain visible at the foot of the dossier")

    # Zoom at an off-center location and confirm that ground point stays
    # pinned to the pointer rather than drifting toward the screen center.
    before_ground = json.loads(
        cdp.evaluate("JSON.stringify(crimocracyCityPreview.groundAt(890, 650))")
    )
    before_zoom = json.loads(cdp.evaluate("JSON.stringify(crimocracyCityPreview.stats())"))["camera"]["zoom"]
    cdp.call(
        "Input.dispatchMouseEvent",
        {
            "type": "mouseWheel",
            "x": 890,
            "y": 650,
            "deltaX": 0,
            "deltaY": -250,
        },
    )
    cdp.next_frame()
    after_ground = json.loads(
        cdp.evaluate("JSON.stringify(crimocracyCityPreview.groundAt(890, 650))")
    )
    after_zoom = json.loads(cdp.evaluate("JSON.stringify(crimocracyCityPreview.stats())"))["camera"]["zoom"]
    check(after_zoom < before_zoom, "mouse wheel should zoom toward the pointer")
    check(
        max(abs(a - b) for a, b in zip(before_ground, after_ground)) < 0.01,
        "zoom must preserve the ground point under the pointer",
    )

    # Right-drag changes the actual camera yaw and preserves an active dossier.
    previous_yaw = json.loads(cdp.evaluate("JSON.stringify(crimocracyCityPreview.stats())"))["camera"]["yaw"]
    for event_type, x, y, buttons in [
        ("mousePressed", 980, 720, 2),
        ("mouseMoved", 1040, 720, 2),
        ("mouseReleased", 1040, 720, 0),
    ]:
        cdp.call(
            "Input.dispatchMouseEvent",
            {
                "type": event_type,
                "x": x,
                "y": y,
                "button": "right",
                "buttons": buttons,
                "clickCount": 1 if event_type != "mouseMoved" else 0,
            },
        )
    cdp.next_frame()
    current_yaw = json.loads(cdp.evaluate("JSON.stringify(crimocracyCityPreview.stats())"))["camera"]["yaw"]
    check(abs(current_yaw - previous_yaw) > 0.1, "right-drag must rotate the camera")
    check(
        cdp.evaluate("document.documentElement.dataset.selectedBusiness") == str(picked_id),
        "orbiting the camera must preserve the selected dossier",
    )

    image = cdp.call("Page.captureScreenshot", {"format": "png", "captureBeyondViewport": False})
    SCREENSHOT.write_bytes(base64.b64decode(image["data"]))

    cdp.call(
        "Emulation.setDeviceMetricsOverride",
        {"width": 1000, "height": 720, "deviceScaleFactor": 1, "mobile": False},
    )
    cdp.next_frame()
    check(
        cdp.evaluate("document.documentElement.dataset.webgl") == "ready",
        "resizing must preserve a working WebGL renderer",
    )
    check(
        cdp.evaluate("document.querySelector('#city-canvas').width") == 1000,
        "canvas pixel size must track viewport resize",
    )
    check(
        cdp.evaluate("document.querySelector('#focus-property').getBoundingClientRect().bottom <= document.querySelector('#dossier').getBoundingClientRect().bottom"),
        "dossier focus action must remain accessible at narrow desktop widths",
    )
    resized_image = cdp.call(
        "Page.captureScreenshot", {"format": "png", "captureBeyondViewport": False}
    )
    NARROW_SCREENSHOT.write_bytes(base64.b64decode(resized_image["data"]))

    cdp.evaluate("document.querySelector('#close-dossier').click()")
    check(
        cdp.evaluate("document.documentElement.dataset.selectedBusiness") == "0",
        "closing the dossier must clear its selection",
    )
    cdp.evaluate(
        """(() => {
            document.querySelector('.property-button').click();
            document.querySelector('#dossier-detail').scrollTop = 160;
            document.querySelectorAll('.property-button')[1].click();
        })()"""
    )
    check(
        cdp.evaluate("document.querySelector('#dossier-detail').scrollTop") == 0,
        "opening another dossier must reset its scroll position",
    )
    cdp.evaluate("document.querySelector('#search').focus()")
    cdp.call(
        "Input.dispatchKeyEvent",
        {"type": "keyDown", "key": "Escape", "code": "Escape", "windowsVirtualKeyCode": 27},
    )
    check(
        cdp.evaluate("document.documentElement.dataset.selectedBusiness") == "0",
        "Escape must close a dossier even while the search input is focused",
    )
    check(
        cdp.evaluate("document.activeElement !== document.querySelector('#search')"),
        "Escape must dismiss search-field focus",
    )
    cdp.evaluate("document.querySelector('#reset-camera').click()")
    cdp.next_frame()
    check(
        cdp.evaluate("document.querySelector('.ward-button.active').textContent.includes('Waterfront')"),
        "reset view must focus the first ward",
    )
    print(
        f"PASS: WebGL2 rendered {initial['buildings']} buildings in "
        f"{initial['drawCalls']} draw calls; directory filtering, ward navigation, "
        f"bounded GPU cache, eviction/re-upload, whole-city overview, GPU picking of business {picked_id}, dossier, cursor-anchored zoom, "
        f"orbit, label decluttering, responsive resize, dismissal, and camera reset work"
    )
    print(f"Captured interacting GUI screenshot: {SCREENSHOT.relative_to(PROJECT)}")
    print(f"Captured narrow GUI screenshot: {NARROW_SCREENSHOT.relative_to(PROJECT)}")
    print(f"Captured city overview screenshot: {OVERVIEW_SCREENSHOT.relative_to(PROJECT)}")


def main() -> None:
    html = PREVIEW / "index.html"
    if not html.is_file():
        raise RuntimeError("Run cargo run --locked --example city_gui_preview first")
    verify_export_disclosure(html.read_text(encoding="utf-8"))
    shutil.rmtree(PROFILE, ignore_errors=True)
    browser = find_browser()
    args = [
        browser,
        "--headless=new",
        "--no-sandbox",
        "--disable-gpu-sandbox",
        "--enable-unsafe-swiftshader",
        "--use-angle=swiftshader",
        "--no-first-run",
        "--no-default-browser-check",
        "--remote-allow-origins=http://localhost",
        "--remote-debugging-port=0",
        "--window-size=1600,950",
        f"--user-data-dir={PROFILE}",
        html.resolve().as_uri(),
    ]
    process = subprocess.Popen(args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        cdp = connect(process)
        try:
            run_checks(cdp)
        finally:
            try:
                cdp.call("Browser.close")
            except (OSError, RuntimeError, websocket.WebSocketException):
                pass
            cdp.connection.close()
    finally:
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
        shutil.rmtree(PROFILE, ignore_errors=True)


if __name__ == "__main__":
    try:
        main()
    except (OSError, AssertionError, RuntimeError, ValueError, websocket.WebSocketException) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        raise SystemExit(1) from error
