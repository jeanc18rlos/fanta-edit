export type CanvasPage = { index: number; name: string; hidden: boolean }

export type CanvasFrame = {
  file: string
  generation: number
  width: number
  height: number
  page: { index: number; name: string }
}

export type CanvasView = {
  status: string
  project?: string
  frame?: CanvasFrame
  pages: CanvasPage[]
}

declare module 'claude-code' {
  interface PluginState {
    'fanta-canvas': { view: Shaped<CanvasView> }
  }
}
