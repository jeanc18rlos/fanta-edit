// The Fanta canvas pane (spec 0008 T4/T5): /fanta-canvas opens a pane that
// shows the current page, re-rendered whenever the project changes on disk.
// The frames come from `fanta-mcp --frames`, which watches the project and
// keeps one PNG fresh in the project's git-ignored previews/ directory. A
// terminal reads that file itself, so no pixels cross the plugin boundary;
// the desktop app draws the page's own SVG with its stylesheets inlined
// (this build's plugin surfaces host no web component, so edits go through
// the chat tools and appear here live).
import { atom, read, update } from 'claude-code'
import type { Hook, PluginOptions, Register } from 'claude-code'

type Api = Parameters<Hook<'command.run'>>[0]

const PANE = 'fanta-canvas'

const view = atom(
  { plugin: 'fanta-canvas', key: 'view' } as const,
  { status: 'Run /fanta-canvas to start.', pages: [] },
  { shape: 'view-2' },
)

/** The Svg element's source cap; larger pages fall back to a pointer. */
const SVG_LIMIT = 131072

/** The current page's SVG with its `<?xml-stylesheet?>` sheets inlined as a
 * `<style>` element — a sandboxed vector frame resolves no relative hrefs. */
async function inlineSvg($: Api, project: string, pageName: string) {
  const manifest = await $.fs.read(`${project}/pages.js`, { as: 'text' })
  const listed = manifest.slice(manifest.indexOf('['), manifest.lastIndexOf(']') + 1)
  const entries: { slug: string; name: string }[] = JSON.parse(listed)
  // pages.js lists visible pages only, so match by name, not index.
  const slug = entries.find(entry => entry.name === pageName)?.slug
  if (!slug) return undefined
  const directory = `${project}/pages/${slug}`
  let svg = await $.fs.read(`${directory}/page.svg`, { as: 'text' })
  let styles = ''
  for (const match of svg.matchAll(/<\?xml-stylesheet[^?]*href="([^"]+)"[^?]*\?>/g)) {
    const parts = `${directory}/${match[1]}`.split('/')
    const resolved: string[] = []
    for (const part of parts) {
      if (part === '..') resolved.pop()
      else if (part !== '.') resolved.push(part)
    }
    styles += (await $.fs.read(resolved.join('/'), { as: 'text' })) + '\n'
  }
  svg = svg.replace(/<\?xml-stylesheet[^?]*\?>\s*/g, '')
  if (styles) {
    svg = svg.replace(/(<svg[^>]*>)/, `$1<style>\n${styles}</style>`)
  }
  return svg
}

// Which watch owns the child: bumping it makes the old for-await loop return,
// which kills its child. Module state dies with the child on hot reload.
let run = 0

async function findProject($: Api, options: PluginOptions) {
  const configured = typeof options.project === 'string' ? options.project : ''
  if (configured) {
    return (await $.fs.exists(`${configured}/fanta.json`)) ? configured : undefined
  }
  const root = await $.session.root()
  if (await $.fs.exists(`${root}/fanta.json`)) return root
  for (const entry of await $.fs.list(root)) {
    if (entry.kind !== 'dir') continue
    const child = `${root}/${entry.name}`
    if (await $.fs.exists(`${child}/fanta.json`)) return child
  }
  return undefined
}

async function watch($: Api, options: PluginOptions, project: string, page: number | undefined) {
  const token = ++run
  let binary = typeof options.mcpBinary === 'string' && options.mcpBinary ? options.mcpBinary : ''
  if (!binary) {
    const env = await $.process.run(['printenv', 'FANTA_MCP_BIN']).catch(() => undefined)
    binary = env?.exitCode === 0 ? env.stdout.trim() : ''
  }
  if (!binary) binary = 'fanta-mcp'

  const argv = [binary, '--frames', project, '--out', `${project}/previews/claude-pane.png`]
  if (page !== undefined) argv.push('--page', String(page))
  await update($, view, current => ({ ...current, status: 'rendering…', project }))
  let rest = ''
  try {
    for await (const chunk of $.process.spawn({ argv })) {
      if (token !== run) return
      if (chunk.stream !== 'stdout') continue
      rest += chunk.text
      const lines = rest.split('\n')
      rest = lines.pop() ?? ''
      for (const line of lines) {
        if (!line.trim()) continue
        let event
        try {
          event = JSON.parse(line)
        } catch {
          continue
        }
        if (event.event === 'frame') {
          await update($, view, current => ({
            ...current,
            status: 'live',
            frame: {
              file: event.out,
              generation: event.generation,
              width: event.width,
              height: event.height,
              page: event.page,
            },
            pages: event.pages ?? current.pages,
          }))
          const svg = await inlineSvg($, project, event.page.name).catch(() => undefined)
          const tooLarge = svg !== undefined && svg.length > SVG_LIMIT
          await update($, view, current => ({
            ...current,
            svg: tooLarge ? undefined : svg,
            svgTooLarge: tooLarge,
          }))
        } else if (event.event === 'error') {
          await update($, view, current => ({ ...current, status: `error: ${event.message}` }))
        }
      }
    }
    if (token === run) {
      await update($, view, current => ({ ...current, status: 'fanta-mcp exited.' }))
    }
  } catch (error) {
    if (token !== run) return
    await update($, view, current => ({
      ...current,
      status:
        `could not run ${binary}: ${error instanceof Error ? error.message : String(error)} — ` +
        'build it with `cargo build -p fanta-mcp`, then put it on PATH, export FANTA_MCP_BIN, ' +
        'or set the plugin’s mcpBinary setting.',
    }))
  }
}

export const register: Register = (on, options) => {
  on('session.start', async ($, e, next) => {
    await $.command.register({
      name: 'fanta-canvas',
      description: 'Open the Fanta canvas pane: the project page as live PNG frames',
    })
    return next(e)
  })

  on('command.run', { command: 'fanta-canvas' }, async $ => {
    const project = await findProject($, options)
    if (!project) {
      return {
        text:
          'No Fanta project found: no fanta.json at the session root or one level down. ' +
          'Set the plugin’s "project" setting to the project directory.',
      }
    }
    await $.ui.open({ id: PANE, title: 'Fanta canvas', focus: true })
    void watch($, options, project, undefined)
    return { text: `Fanta canvas pane opened on ${project}.` }
  })

  on('ui.select', { element: 'page' }, async ($, e, next) => {
    const current = await read($, view)
    if (current.project) void watch($, options, current.project, Number(e.value))
    return next(e)
  })

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const elements = $.ui.resolve(e)
    const { Box, Text, Select } = elements
    const state = await read($, view)
    const body = Math.max(10, Math.min(e.props.bodyColumns, 255))

    let frame = null
    if (state.frame && 'Svg' in elements && (state.svg || state.svgTooLarge)) {
      const { Svg } = elements
      frame = state.svg ? (
        <Svg
          source={state.svg}
          alt={`Fanta page ${state.frame.page.name}`}
          isInteractive
        />
      ) : (
        <Text dimColor>
          {'This page is too large for the pane; serve it with script/serve-project instead.'}
        </Text>
      )
    } else if (state.frame && 'Image' in elements) {
      const { Image } = elements
      // A terminal cell is roughly twice as tall as it is wide.
      const rows = Math.max(
        1,
        Math.min(
          255,
          Math.round(body * (state.frame.height / state.frame.width) * 0.5),
          (e.viewport?.rows ?? 40) - 4,
        ),
      )
      frame = (
        <Image
          key="frame"
          source={{ file: state.frame.file, format: 'png', generation: state.frame.generation }}
          columns={body}
          rows={rows}
          alt={`Fanta page ${state.frame.page.name} (${state.frame.width}×${state.frame.height})`}
        />
      )
    }

    return (
      <Box flexDirection="column">
        <Text>
          {state.frame ? `${state.frame.page.name} — ${state.status}` : state.status}
        </Text>
        {state.pages.length > 1 && (
          <Select
            key="page"
            label="Page"
            options={state.pages.map(page => ({
              value: String(page.index),
              label: page.hidden ? `${page.name} (hidden)` : page.name,
            }))}
            value={String(state.frame?.page.index ?? 0)}
          />
        )}
        {frame ??
          (state.frame ? (
            <Text dimColor>{'Preparing the vector view…'}</Text>
          ) : (
            <Text dimColor>{'Waiting for the first frame…'}</Text>
          ))}
      </Box>
    )
  })
}
