import { useState } from "react";

import { readStoredBool, writeStoredBool } from "./settings";

// localStorage-backed boolean React state. Thin wrapper around the
// shared `lib/settings` helpers so React surfaces (settings panes,
// UpdatePromptCard) and non-React readers (UpdateContext) can't drift
// on encoding — both sides go through `readStoredBool` /
// `writeStoredBool`.
export function useStoredBool(
  key: string,
  initial: boolean,
): [boolean, (v: boolean) => void] {
  const [value, setValue] = useState<boolean>(() =>
    readStoredBool(key, initial),
  );
  const set = (v: boolean) => {
    setValue(v);
    writeStoredBool(key, v);
  };
  return [value, set];
}
