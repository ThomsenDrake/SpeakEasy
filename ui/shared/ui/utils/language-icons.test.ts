import deviconCatalog from "devicon/devicon.json";
import { describe, expect, it } from "vitest";

import { DEVICON_NAMES } from "./language-icons";

describe("DEVICON_NAMES", () => {
  it("matches the icon names from devicon/devicon.json", () => {
    const packageNames = new Set(deviconCatalog.map((icon) => icon.name));

    const missing = [...packageNames].filter((n) => !DEVICON_NAMES.has(n));
    const extra = [...DEVICON_NAMES].filter((n) => !packageNames.has(n));

    expect(
      missing,
      "Icons in devicon package but missing from DEVICON_NAMES",
    ).toEqual([]);
    expect(
      extra,
      "Icons in DEVICON_NAMES but missing from devicon package",
    ).toEqual([]);
  });
});
