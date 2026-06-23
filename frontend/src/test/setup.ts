// jsdom has no ResizeObserver. This mock fires the callback immediately on
// observe() with the element's current clientWidth so virtualization math runs
// deterministically in tests.
class ResizeObserverMock {
  private cb: ResizeObserverCallback
  constructor(cb: ResizeObserverCallback) { this.cb = cb }
  observe(el: Element) {
    const width = (el as HTMLElement).clientWidth || 0
    this.cb([{ contentRect: { width } } as ResizeObserverEntry], this as unknown as ResizeObserver)
  }
  unobserve() {}
  disconnect() {}
}
globalThis.ResizeObserver = ResizeObserverMock as unknown as typeof ResizeObserver
