import { expect, test } from 'claude-code/testing'

const PROPS = {
  title: 'Fanta canvas',
  isFocused: false,
  bodyColumns: 40,
  placement: 'dock',
  scroll: { offset: 0, bodyRows: 20 },
  view: {},
} as const

for (const surface of ['terminal', 'desktop'] as const) {
  test(`the ${surface} pane waits for a frame before drawing one`, async ($, on) => {
    on('ui.open', () => ({ value: { isPlaced: true } }))
    on('command.register', () => ({ value: undefined }))
    on('session.start', () => ({ cwd: '/tmp' }))
    await $.session.start({ cwd: '/tmp', surface, isInteractive: true })
    const ui = await $.ui.mount({
      plugin: 'fanta-canvas',
      surface,
      component: 'Pane',
      requestId: 'fanta-canvas',
      props: PROPS,
    })
    const waiting = await ui.find({ type: 'Text', text: /start|Waiting/ })
    expect(waiting == null).toBe(false)
    await ui.unmount()
  })
}

test('the command explains itself when no project is found', async ($, on) => {
  on('ui.open', () => ({ value: { isPlaced: true } }))
  on('command.register', () => ({ value: undefined }))
  on('session.start', () => ({ cwd: '/tmp' }))
  on('session.root', () => ({ value: '/tmp' }))
  on('fs.exists', () => ({ value: false }))
  on('fs.list', () => ({ value: [] }))
  await $.session.start({ cwd: '/tmp', surface: 'terminal', isInteractive: true })
  const ran = await $.command.run({ command: 'fanta-canvas', args: '' })
  expect(String(ran?.text ?? '')).toContain('No Fanta project found')
})
