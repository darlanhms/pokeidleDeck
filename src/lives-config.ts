import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

type Reward = "xp" | "capture" | "lure" | null;
type Account = { id: string; name: string; redeemTwitch: Reward; redeemKick: Reward };
type Form = { twitch: string; kick: string; twitchSlots: number; kickSlots: number; claimChest: boolean; accounts: Account[] };

const REWARDS: [Reward, string][] = [
  [null, "Não resgatar"],
  ["xp", "+15% XP (300 pontos)"],
  ["capture", "+15% Captura (500 pontos)"],
  ["lure", "+15% Shiny (1000 pontos)"],
];

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
let accounts: Account[] = [];

function rewardSelect(value: Reward, label: string): HTMLSelectElement {
  const select = document.createElement("select");
  select.ariaLabel = label;
  for (const [reward, text] of REWARDS) select.append(new Option(text, reward ?? "", false, reward === value));
  return select;
}

function slots(id: string): number {
  return Math.max(0, Math.min(10, Math.trunc(Number($<HTMLInputElement>(id).value) || 0)));
}

invoke<Form>("lives_config_get").then((form) => {
  $<HTMLTextAreaElement>("twitch").value = form.twitch;
  $<HTMLTextAreaElement>("kick").value = form.kick;
  $<HTMLInputElement>("twitch-slots").value = String(form.twitchSlots);
  $<HTMLInputElement>("kick-slots").value = String(form.kickSlots);
  $<HTMLInputElement>("chest").checked = form.claimChest;
  accounts = form.accounts;
  const grid = $("redeem");
  for (const account of accounts) {
    grid.append(
      Object.assign(document.createElement("strong"), { textContent: account.name }),
      rewardSelect(account.redeemKick, `${account.name}: resgatar com os pontos da Kick`),
      rewardSelect(account.redeemTwitch, `${account.name}: resgatar com os pontos da Twitch`),
    );
  }
});

$("form").addEventListener("submit", (e) => {
  e.preventDefault();
  const selects = [...$("redeem").querySelectorAll("select")];
  const value = (select: HTMLSelectElement) => (select.value || null) as Reward;
  const form: Form = {
    twitch: $<HTMLTextAreaElement>("twitch").value,
    kick: $<HTMLTextAreaElement>("kick").value,
    twitchSlots: slots("twitch-slots"),
    kickSlots: slots("kick-slots"),
    claimChest: $<HTMLInputElement>("chest").checked,
    accounts: accounts.map((a, i) => ({ ...a, redeemKick: value(selects[i * 2]), redeemTwitch: value(selects[i * 2 + 1]) })),
  };
  invoke("lives_config_set", { form });
});
$("cancel").addEventListener("click", () => getCurrentWindow().close());
