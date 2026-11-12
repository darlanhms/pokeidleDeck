// O recompra.js num jogo de mentira: socket falso, catálogo falso, e o deck chamando configure/summary.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";

const SOURCE = fs.readFileSync(new URL("../src-tauri/scripts/recompra.js", import.meta.url), "utf8");
const GOLDEN = 70070;
const ULTIMATE = 204;
const MIRROR = [
  { id: 202, name: "Ultra Potion", category: "heal", npcPrice: 400, healAmount: 400 },
  { id: ULTIMATE, name: "Ultimate Potion", category: "heal", npcPrice: 1500, healAmount: 3000 },
  { id: 300, name: "Revive", category: "revive", npcPrice: 2000 },
];

function game({ items }) {
  const timers = [];
  const sent = [];
  class FakeSocket {
    constructor(url) {
      this.url = url;
      this.readyState = 1;
      this.listeners = [];
    }
    addEventListener(type, listener) {
      if (type === "message") this.listeners.push(listener);
    }
    send(data) {
      sent.push(JSON.parse(data));
    }
    emit(message) {
      for (const listener of this.listeners) listener({ data: JSON.stringify(message) });
    }
  }
  const page = {
    WebSocket: FakeSocket,
    location: { href: "https://pokeidle.io/app", host: "pokeidle.io" },
    URL,
    setInterval: (f) => timers.push(f),
    fetch: async () => ({ json: async () => ({ items: MIRROR }) }),
  };
  page.window = page;
  page.top = page;
  vm.createContext(page);
  vm.runInContext(SOURCE, page);
  const socket = new page.WebSocket("wss://pokeidle.io/ws");
  socket.emit({
    t: "welcome",
    catalogoBolas: [{ id: 4, nome: "Ultra Ball", priceGold: 300, compravel: true }],
    itensNossos: [{ id: GOLDEN, name: "Golden Potion", category: "heal", npcPrice: 5000, healPct: 0.5 }],
    estado: { gold: 50_000_000, balls: { 4: 100_000 }, items, automation: { potionIds: [] }, activeId: 1, nick: "teste", pokemons: [{ id: 1, maxHp: 1000 }] },
  });
  const tick = async () => {
    for (const f of timers) f();
    await new Promise((r) => setTimeout(r, 0));
  };
  const buys = () => sent.filter((m) => m.t === "shop.buy");
  return { deck: page.__deckRecompra, tick, buys };
}

const configure = (potions) => ({
  on: true,
  balls: { on: false, id: 4, keep: 0, below: 0 },
  revives: { on: false, id: 0, keep: 0, below: 0 },
  potions: { on: true, ...potions },
  potionFirst: false,
  goldFloor: 0,
});

test("Golden escolhida: compra por quantidade dela, ignorando as outras poções na mochila", async () => {
  const { deck, tick, buys } = game({ items: { [ULTIMATE]: 6000, [GOLDEN]: 10 } });
  deck.configure(configure({ id: GOLDEN, keep: 500, below: 100 }));
  await tick();
  await tick();
  assert.deepEqual(buys(), [{ t: "shop.buy", kind: "item", id: GOLDEN, qty: 490 }]);
});

test("Golden acima do 'recomprar abaixo de' não compra", async () => {
  const { deck, tick, buys } = game({ items: { [GOLDEN]: 150 } });
  deck.configure(configure({ id: GOLDEN, keep: 500, below: 100 }));
  await tick();
  await tick();
  assert.deepEqual(buys(), []);
});

test("Outra poção escolhida: a Golden não entra na soma de 'todas as poções'", async () => {
  const { deck, tick, buys } = game({ items: { [ULTIMATE]: 100, [GOLDEN]: 5000 } });
  deck.configure(configure({ id: ULTIMATE, keep: 5000, below: 2000 }));
  await tick();
  await tick();
  assert.deepEqual(buys(), [{ t: "shop.buy", kind: "item", id: ULTIMATE, qty: 4900 }]);
});

test("O resumo lista a Golden (marcada) para a janela poder oferecer", async () => {
  const { deck, tick } = game({ items: { [GOLDEN]: 7 } });
  deck.configure(configure({ id: GOLDEN, keep: 0, below: 0 }));
  await tick();
  const summary = deck.summary();
  const golden = summary.potions.find((p) => p.id === GOLDEN);
  assert.equal(golden.golden, true);
  assert.equal(golden.have, 7);
  assert.equal(summary.potionId, GOLDEN);
  assert.equal(summary.potions.filter((p) => p.golden).length, 1);
});
