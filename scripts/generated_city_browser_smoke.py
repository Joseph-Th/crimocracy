"""Real WebGL2 smoke for a procedurally generated campaign city.

Generate it first:
    cargo run --locked --example city_gui_preview -- --generate --seed 12 --city-size medium
    python scripts/generated_city_browser_smoke.py

Uses the same self-contained browser launcher and test helpers as the authored
preview smoke, but derives its expected ward and property IDs from the public
export instead of assuming the 8-ward fixture and its 96 sample properties.
"""
from __future__ import annotations

import base64
import json
import re
import sys
import time

# Import the reusable browser test without writing Python bytecode files into
# the user's working tree.
sys.dont_write_bytecode = True
import city_preview_browser_smoke as base


def read_public_scene() -> dict:
    html = (base.PREVIEW / "index.html").read_text(encoding="utf-8")
    match = re.search(r'<script id="city-preview-data" type="application/json">(.*?)</script>',
                      html, re.DOTALL)
    base.check(match is not None, "generated preview must embed a single public scene")
    scene = json.loads(match.group(1))
    districts = scene["districts"]
    dossiers = scene["dossiers"]
    base.check(4 <= len(districts) <= 24, "generated scene must have multiple districts")
    base.check(len(dossiers) >= 8 * len(districts), "generated scene must contain modeled shops")
    public_ids = {int(entry["id"]) for entry in dossiers}
    encoded_ids = set()
    for district in districts:
        for batch in district["buildings"]:
            raw = base64.b64decode(batch["bytes"], validate=True)
            base.check(len(raw) == int(batch["instance_count"]) * 32, "invalid GPU instance stride")
            encoded_ids.update(int.from_bytes(raw[pos:pos + 4], "little")
                               for pos in range(24, len(raw), 32)
                               if int.from_bytes(raw[pos:pos + 4], "little"))
    base.check(public_ids == encoded_ids, "GPU pick buffer contains a non-public business ID")
    for forbidden in ("business_owner", "police_presence", "case_witness", "player_organization"):
        base.check(forbidden not in html, f"private key {forbidden} leaked into public HTML")
    return scene


def check_browser(cdp: base.DevTools, scene: dict) -> None:
    for _ in range(100):
        if cdp.evaluate("document.documentElement.dataset.webgl") == "ready":
            break
        time.sleep(.1)
    else:
        raise AssertionError("generated city never reached a ready WebGL2 renderer")
    stats = json.loads(cdp.evaluate("JSON.stringify(crimocracyCityPreview.stats())"))
    base.check(stats["drawCalls"] > 0 and stats["buildings"] > 0,
               "generated streets and buildings must render")
    base.check(cdp.evaluate("document.querySelectorAll('.ward-button').length") == len(scene["districts"]),
               "ward controls must match generated districts")
    count = cdp.evaluate("document.querySelectorAll('.property-button').length")
    base.check(count == min(len(scene["dossiers"]), 200), "directory count must match public properties")
    first = scene["dossiers"][0]
    base.check(cdp.evaluate("document.querySelector('.property-button').textContent.length") > 0,
               "generated business directory must display usable identities")
    cdp.evaluate(f"crimocracyCityPreview.focusOnProperty({int(first['id'])})")
    cdp.next_frame()
    cdp.evaluate(f"crimocracyCityPreview.selectBusiness({int(first['id'])})")
    base.check(cdp.evaluate("document.querySelector('#dossier-name').textContent") == first["name"],
               "actual selected business must populate the public dossier")
    base.check(cdp.evaluate("document.documentElement.dataset.selectedBusiness") == str(first["id"]),
               "property selection must preserve the persistent generated ID")
    last = int(scene["districts"][-1]["id"])
    cdp.evaluate(f"crimocracyCityPreview.showWard({last})")
    cdp.next_frame()
    base.check(cdp.evaluate("document.querySelector('.ward-button.active') !== null"),
               "navigation must activate a generated ward")
    cdp.evaluate("document.querySelector('#city-overview').click()")
    cdp.next_frame()
    overview = json.loads(cdp.evaluate("JSON.stringify(crimocracyCityPreview.stats())"))
    base.check(overview["visibleDistricts"] == len(scene["districts"]),
               "overview must cover all generated districts")
    base.check(overview["buildings"] > 0, "city overview must render modeled geometry")
    screenshot = cdp.call("Page.captureScreenshot", {
        "format": "png", "captureBeyondViewport": False})
    output = base.PREVIEW / "generated-overview.png"
    output.write_bytes(base64.b64decode(screenshot["data"]))
    print(f"PASS: generated city WebGL2 rendered {len(scene['districts'])} districts, "
          f"{len(scene['dossiers'])} public shops; selection, navigation, overview and GPU IDs verified")
    print(f"Screenshot: {output.relative_to(base.PROJECT)}")


def main() -> None:
    scene = read_public_scene()
    with base.headless_browser(base.PREVIEW / "index.html") as cdp:
        check_browser(cdp, scene)


if __name__ == "__main__":
    try:
        main()
    except (OSError, AssertionError, RuntimeError, ValueError, base.websocket.WebSocketException) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        raise SystemExit(1) from error
