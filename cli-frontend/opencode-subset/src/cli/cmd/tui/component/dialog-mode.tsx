import { createMemo } from "solid-js"
import { useLocal } from "@tui/context/local"
import { DialogSelect } from "@tui/ui/dialog-select"
import { useDialog } from "@tui/ui/dialog"

const MODE_DESCRIPTIONS: Record<string, string> = {
  Plan: "Read project, analyze, plan tasks (read-only)",
  Lite: "Plan + code + verify + safety (daily dev)",
  Full: "All 29 modules: governance + evolution + evidence",
  Test: "Fix bugs + verify + soak test (90%→100%)",
}

export function DialogMode() {
  const local = useLocal()
  const dialog = useDialog()

  const options = createMemo(() =>
    local.mode.list().map((item) => ({
      value: item,
      title: item,
      description: MODE_DESCRIPTIONS[item] ?? "",
    })),
  )

  return (
    <DialogSelect
      title="Select mode"
      current={local.mode.current()}
      options={options()}
      onSelect={(option) => {
        local.mode.set(option.value)
        dialog.clear()
      }}
    />
  )
}
