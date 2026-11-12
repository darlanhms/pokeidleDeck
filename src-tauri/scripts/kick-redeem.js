// Resgata um bônus de um canal da Kick, rodando num player da Kick da conta. Copiado do original
// (BulkRedeemDialog, resgate de um da Kick).
(async a => {
 const head = { Accept: 'application/json', 'x-app-platform': 'web', Authorization: 'Bearer ' + a.token };
 const ch = a.api + '/api/v2/channels/' + encodeURIComponent(a.channel);
 const get = async url => { try { const r = await fetch(url, { cache: 'no-cache', credentials: 'include', headers: head });
  return { status: r.status, json: r.ok ? await r.json().catch(() => null) : null }; }
  catch (e) { return { status: 0, json: null, fail: (e && e.message) || String(e) }; } };
 const pick = all => a.words.map(w => { const word = new RegExp(w, 'i');
  return all.find(r => word.test(r.title) && /pokeidle/i.test(r.title + ' ' + (r.prompt || r.about || ''))) ?? null; });
 try {
  const abc = '0123456789ABCDEFGHJKMNPQRSTVWXYZ'; let t = Date.now(), id = '';
  for (let i = 0; i < 10; i++) { id = abc[t % 32] + id; t = Math.floor(t / 32); }
  for (let i = 0; i < 16; i++) id += abc[Math.floor(Math.random() * 32)];
  const r = await fetch(ch + '/rewards/' + encodeURIComponent(a.rewardId) + '/redeem', { method: 'POST', cache: 'no-cache',
   credentials: 'include', headers: { ...head, 'Content-Type': 'application/json' }, body: JSON.stringify({ transaction_id: id, message: '' }) });
  if (r.ok) return { redeemed: true };
  const j = await r.json().catch(() => null);
  return { error: r.status + (typeof j?.message === 'string' && j.message ? ' ' + j.message.slice(0, 80) : '') };
 } catch (e) { return { error: 'sem resposta da Kick: ' + (e && e.message || e) }; }
})(__ARGS__).then(o => JSON.stringify(o))
