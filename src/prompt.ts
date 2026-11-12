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
  await fitContent();
  input.focus();
  input.select();
}

// A altura depende de quantas linhas o texto ocupa; a janela segue o conteúdo em vez de cortar os botões.
async function fitContent() {
  const form = document.querySelector<HTMLElement>("#form")!;
  await invoke("prompt_fit", { height: form.getBoundingClientRect().height });
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
