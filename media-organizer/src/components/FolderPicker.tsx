import { open } from "@tauri-apps/plugin-dialog";
import { Button, FolderIcon } from "./ui";

/** A read-only path field with a native folder browser beside it. */
export function FolderPicker({
  value,
  onChange,
  placeholder = "No folder chosen",
  title = "Choose a folder",
  disabled,
}: {
  value: string;
  onChange: (path: string) => void;
  placeholder?: string;
  title?: string;
  disabled?: boolean;
}) {
  async function pick() {
    const selected = await open({
      directory: true,
      multiple: false,
      title,
      defaultPath: value || undefined,
    });
    if (typeof selected === "string" && selected.length > 0) {
      onChange(selected);
    }
  }

  return (
    <div className="flex min-w-0 items-center gap-2">
      <div
        className="min-w-0 flex-1 truncate rounded-lg bg-zinc-950/60 px-3 py-2 text-sm ring-1 ring-white/10"
        title={value || placeholder}
      >
        {value ? (
          <span className="text-zinc-200">{value}</span>
        ) : (
          <span className="text-zinc-600">{placeholder}</span>
        )}
      </div>
      <Button onClick={pick} disabled={disabled}>
        <FolderIcon />
        Browse
      </Button>
    </div>
  );
}
