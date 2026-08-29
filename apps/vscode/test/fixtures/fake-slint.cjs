const fs = require('node:fs')

// Test double for the slint CLI used by extension.test.ts. Configuration comes
// from FAKE_SLINT_* environment variables, which reach the child because the
// extension passes process.env through:
//
//   FAKE_SLINT_RECORD      file the received argv/API-key env is appended to, one JSON line per invocation
//   FAKE_SLINT_ENVELOPE    file whose contents are written to stdout on the first invocation
//   FAKE_SLINT_ENVELOPE_2  file used from the second invocation on (static pass, then model pass)
//   FAKE_SLINT_DELAY_MS    sleep before answering
//   FAKE_SLINT_EXIT        exit code to answer with
//   FAKE_SLINT_HANG        stay alive until killed, then leave a marker at FAKE_SLINT_KILLED

const record = process.env.FAKE_SLINT_RECORD
let invocation = 0
if (record) {
  fs.appendFileSync(
    record,
    `${JSON.stringify({
      argv: process.argv.slice(2),
      envApiKey: process.env.SLINT_EDITOR_API_KEY ?? null,
    })}\n`,
  )
  invocation = fs.readFileSync(record, 'utf8').trim().split('\n').length
}

const finish = () => {
  const envelope =
    invocation <= 1
      ? process.env.FAKE_SLINT_ENVELOPE
      : (process.env.FAKE_SLINT_ENVELOPE_2 ?? process.env.FAKE_SLINT_ENVELOPE)
  if (envelope) process.stdout.write(fs.readFileSync(envelope, 'utf8'))
  process.exit(Number(process.env.FAKE_SLINT_EXIT || 0))
}

if (process.env.FAKE_SLINT_HANG === '1') {
  const killed = process.env.FAKE_SLINT_KILLED
  process.on('SIGTERM', () => {
    if (killed) fs.writeFileSync(killed, 'killed')
    process.exit(143)
  })
  setInterval(() => {}, 1000)
} else {
  const delay = Number(process.env.FAKE_SLINT_DELAY_MS || 0)
  if (delay > 0) {
    setTimeout(finish, delay)
  } else {
    finish()
  }
}
