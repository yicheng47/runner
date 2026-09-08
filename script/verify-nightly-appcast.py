#!/usr/bin/env python3

import base64
from pathlib import Path
import sys
import xml.etree.ElementTree as ET


def verify(appcast, dmg, version, stamp):
    sparkle = "{http://www.andymatuschak.org/xml-namespaces/sparkle}"
    release_url = "https://github.com/yicheng47/runner/releases/tag/nightly"
    items = ET.parse(appcast).findall("./channel/item")
    if len(items) != 1 or len(items[0].findall("enclosure")) != 1:
        raise ValueError("nightly appcast must contain exactly one item and enclosure")
    item = items[0]
    enclosure = item.find("enclosure")
    expected = {
        "version": stamp,
        "shortVersionString": version,
        "hardwareRequirements": "arm64",
        "fullReleaseNotesLink": release_url,
    }
    for field, value in expected.items():
        if item.findtext(sparkle + field) != value:
            raise ValueError(f"unexpected appcast {field}")
    if item.findtext("link") != release_url:
        raise ValueError("appcast link must point to the nightly release")
    dmg = Path(dmg)
    expected_url = f"https://github.com/yicheng47/runner/releases/download/nightly/{dmg.name}"
    if enclosure.get("url") != expected_url:
        raise ValueError("appcast enclosure must point to the expected nightly DMG")
    if int(enclosure.get("length", "0")) != dmg.stat().st_size:
        raise ValueError("appcast enclosure length differs from the DMG")
    signature = base64.b64decode(enclosure.get(sparkle + "edSignature", ""), validate=True)
    if len(signature) != 64:
        raise ValueError("appcast enclosure is missing an EdDSA signature")


if __name__ == "__main__":
    verify(*sys.argv[1:])
