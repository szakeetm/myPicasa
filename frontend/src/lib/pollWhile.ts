/** Run one read at a time. The reader returns false after its terminal result. */
export function pollWhile(read: (signal: AbortSignal) => Promise<boolean>, delay: number) {
  const controller = new AbortController();
  let timer: ReturnType<typeof setTimeout> | undefined;

  async function tick() {
    const keepPolling = await read(controller.signal);
    if (keepPolling && !controller.signal.aborted) {
      timer = setTimeout(() => void tick(), delay);
    }
  }

  void tick();
  return () => {
    controller.abort();
    clearTimeout(timer);
  };
}
