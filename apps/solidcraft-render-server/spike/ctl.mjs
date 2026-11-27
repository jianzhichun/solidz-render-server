// One control-channel request to a render server (docs/control-protocol.md "Token"): prints the reply.
//   node ctl.mjs PORT TOKEN METHOD ['{"json":"params"}']
// e.g. node ctl.mjs 51234 $TOKEN ui.handles '{"prefix":"tab:"}'
import net from 'node:net'

const [port, token, method, params = '{}'] = process.argv.slice(2)
if (!port || !token || !method) {
  console.error('usage: node ctl.mjs PORT TOKEN METHOD [PARAMS_JSON]')
  process.exit(2)
}
const sock = net.connect(Number(port), '127.0.0.1', () => {
  sock.write(JSON.stringify({ id: 1, method, params: JSON.parse(params), token }) + '\n')
})
let buf = ''
sock.on('data', (d) => {
  buf += d
  const nl = buf.indexOf('\n')
  if (nl >= 0) {
    console.log(JSON.stringify(JSON.parse(buf.slice(0, nl)), null, 2))
    sock.end()
  }
})
sock.on('error', (e) => {
  console.error(String(e))
  process.exit(1)
})
setTimeout(() => {
  console.error('no reply in 30 s')
  process.exit(1)
}, 30_000).unref()
