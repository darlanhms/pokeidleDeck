import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type PromptInfo = { title: string; label: string; value: string };

const input = document.querySelector<HTMLInputElement>("#value")!;
const label = document.querySelector<HTMLLabelElement>("#label")!;

async function load() {
  const info = await invoke<PromptInfo | null>("prompt_info");
  if (!info) return;
  document.title = info.title;
  label.textContent = info.label;
  input.value = info.value;
  input.focus();
  input.select();
}

const submit = (value: string | null) => invoke("prompt_submit", { value });

document.querySelector("#form")!.addEventListener("submit", (e) => {
  e.preventDefault();
  submit(input.value.trim() || null);
});
document.querySelector("#cancel")!.addEventListener("click", () => submit(null));
addEventListener("keydown", (e) => {
  if (e.key === "Escape") submit(null);
});
listen("prompt-changed", load);
load();
