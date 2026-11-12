// "Menos animações": a página passa a ver o "reduzir movimento" ligado, e o próprio jogo desliga as
// animações que rodam sem parar. O original fazia isso pelo CDP (Emulation.setEmulatedMedia); aqui o
// jogo lê pelo matchMedia (a cada uso) e por ~110 regras @media do CSS, então as duas coisas são trocadas.
(() => {
  if (window.top !== window || window.__deckMotion) return;
  const REDUCE = /prefers-reduced-motion\s*:\s*reduce/;
  const NO_PREFERENCE = /prefers-reduced-motion\s*:\s*no-preference/;
  let reduce = __DECK_REDUCE_MOTION__;
  const original = new WeakMap();

  const nativeMatchMedia = window.matchMedia.bind(window);
  window.matchMedia = function matchMedia(query) {
    const list = nativeMatchMedia(query);
    const text = String(query);
    const wantsReduce = REDUCE.test(text);
    if (!wantsReduce && !NO_PREFERENCE.test(text)) return list;
    return new Proxy(list, {
      get(target, prop) {
        if (prop === 'matches') return wantsReduce ? reduce || target.matches : !reduce && target.matches;
        const value = Reflect.get(target, prop, target);
        return typeof value === 'function' ? value.bind(target) : value;
      },
    });
  };

  function rewrite(rules) {
    for (const rule of rules) {
      if (typeof CSSMediaRule !== 'undefined' && rule instanceof CSSMediaRule) {
        const text = original.has(rule) ? original.get(rule) : rule.media.mediaText;
        if (REDUCE.test(text) || NO_PREFERENCE.test(text)) {
          original.set(rule, text);
          const wanted = reduce ? (REDUCE.test(text) ? 'all' : 'not all') : text;
          if (rule.media.mediaText !== wanted) rule.media.mediaText = wanted;
        }
      }
      if (rule.cssRules) rewrite(rule.cssRules);
    }
  }

  function apply() {
    for (const sheet of document.styleSheets) {
      let rules;
      try {
        rules = sheet.cssRules;
      } catch {
        continue;
      }
      rewrite(rules);
    }
  }

  // Folhas que chegam depois (o <link> do estilo.css termina de carregar depois deste script).
  document.addEventListener('load', (e) => {
    if (e.target instanceof HTMLLinkElement || e.target instanceof HTMLStyleElement) apply();
  }, true);
  document.addEventListener('DOMContentLoaded', apply);
  addEventListener('load', apply);

  Object.defineProperty(window, '__deckMotion', {
    value: Object.freeze({
      set(on) {
        reduce = !!on;
        apply();
        return reduce;
      },
      get: () => reduce,
    }),
  });
})();
