// Saldo e resgate automático da Kick, rodando no player da live (copiado do original, PointsRedeemer.KickScript).
(async a => {
 const call = (path, init) => fetch(a.api + '/api/v2/channels/' + encodeURIComponent(a.channel) + path,
  { cache: 'no-cache', credentials: 'include', ...init, headers: { Accept: 'application/json', 'x-app-platform': 'web',
   Authorization: 'Bearer ' + a.token, ...(init && init.body ? { 'Content-Type': 'application/json' } : {}) } });
 try {
  let r = await call('/points');
  if (!r.ok) return { error: 'saldo: erro ' + r.status };
  const points = (await r.json())?.data?.points;
  if (typeof points !== 'number') return { error: 'saldo: sem os pontos na resposta' };
  r = await call('/rewards');
  if (!r.ok) return { points, error: 'bônus: erro ' + r.status };
  const word = new RegExp(a.word, 'i');
  // Pacotes do mesmo bônus (1h a 10h): o maior que o saldo paga; nenhum cabe, o mais barato.
  const all = ((await r.json())?.data || []).filter(x => x.is_enabled && word.test(x.title || '')
   && /pokeidle/i.test((x.title || '') + ' ' + (x.description || ''))).sort((x, y) => x.cost - y.cost);
  const open = all.filter(x => !x.is_paused);
  const reward = open.filter(x => x.cost <= points).pop() ?? open[0] ?? all[0];
  if (!reward) return { points, error: 'canal sem o bônus' };
  const out = { points, cost: reward.cost, title: reward.title, paused: !!reward.is_paused };
  if (!a.spend || reward.is_paused || points < reward.cost || (a.refund !== null && points >= a.refund)) return out;
  const abc = '0123456789ABCDEFGHJKMNPQRSTVWXYZ'; let t = Date.now(), id = '';
  for (let i = 0; i < 10; i++) { id = abc[t % 32] + id; t = Math.floor(t / 32); }
  for (let i = 0; i < 16; i++) id += abc[Math.floor(Math.random() * 32)];
  r = await call('/rewards/' + reward.id + '/redeem', { method: 'POST', body: JSON.stringify({ transaction_id: id, message: '' }) });
  return { ...out, redeemed: r.ok, status: r.status };
 } catch (e) { return { error: 'sem resposta da Kick: ' + (e && e.message || e) }; }
})(__ARGS__).then(o => JSON.stringify(o))
