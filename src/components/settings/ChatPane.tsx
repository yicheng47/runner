// Chat pane — defaults for direct chats.

import { useEffect, useState } from "react";

import { api } from "../../lib/api";
import {
  readDefaultChatRuntime,
  writeDefaultChatRuntime,
} from "../../lib/settings";
import { StyledSelect } from "../ui/StyledSelect";
import { PaneHeader, SettingsCard, SettingsRow } from "./shared";

export function ChatPane() {
  const [runtimes, setRuntimes] = useState<
    { name: string; display_name: string; command: string }[]
  >([]);
  const [defaultRuntime, setDefaultRuntimeState] = useState<string>(() =>
    readDefaultChatRuntime(),
  );

  useEffect(() => {
    let cancelled = false;
    void api.runtime
      .list()
      .then((rows) => {
        if (cancelled) return;
        setRuntimes(rows);
        const stored = readDefaultChatRuntime();
        if (stored && !rows.some((runtime) => runtime.name === stored)) {
          setDefaultRuntimeState("");
          writeDefaultChatRuntime("");
        }
      })
      .catch(() => {
        if (!cancelled) setRuntimes([]);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const setDefaultRuntime = (runtime: string) => {
    setDefaultRuntimeState(runtime);
    writeDefaultChatRuntime(runtime);
  };

  return (
    <>
      <PaneHeader title="Chat" subtitle="Defaults for direct chats." />
      <SettingsCard>
        <SettingsRow
          label="Default runtime"
          sub="Pre-selected in Start Chat's Direct mode."
        >
          <StyledSelect
            value={defaultRuntime}
            options={[
              { value: "", label: "First available" },
              ...runtimes.map((runtime) => ({
                value: runtime.name,
                label: runtime.display_name,
              })),
            ]}
            onChange={setDefaultRuntime}
          />
        </SettingsRow>
      </SettingsCard>
    </>
  );
}
