export type AgentBrowserStreamFrame = {
  type: "frame";
  data: string;
  metadata?: {
    deviceWidth?: number;
    deviceHeight?: number;
    pageScaleFactor?: number;
    offsetTop?: number;
    scrollOffsetX?: number;
    scrollOffsetY?: number;
  };
};

export type AgentBrowserStreamClientOptions = {
  url: string;
  onFrame: (frame: AgentBrowserStreamFrame) => void;
  onError?: (message: string) => void;
  onOpen?: () => void;
  onClose?: () => void;
};

export class AgentBrowserStreamClient {
  private socket: WebSocket | null = null;
  private closed = false;

  constructor(private readonly options: AgentBrowserStreamClientOptions) {}

  connect() {
    this.closed = false;
    const socket = new WebSocket(this.options.url);
    this.socket = socket;
    socket.addEventListener("open", () => this.options.onOpen?.());
    socket.addEventListener("close", () => {
      if (!this.closed) this.options.onClose?.();
    });
    socket.addEventListener("error", () => {
      this.options.onError?.("Browser stream connection failed.");
    });
    socket.addEventListener("message", (event) => {
      try {
        const payload = typeof event.data === "string" ? JSON.parse(event.data) : null;
        if (!payload || payload.type !== "frame" || typeof payload.data !== "string") return;
        this.options.onFrame(payload as AgentBrowserStreamFrame);
      } catch {
        this.options.onError?.("Browser stream returned invalid frame data.");
      }
    });
  }

  sendMouse(eventType: "mousePressed" | "mouseReleased" | "mouseMoved", x: number, y: number, button: "left" | "right" | "middle" = "left") {
    this.send({
      type: "input_mouse",
      eventType,
      x,
      y,
      button,
      clickCount: eventType === "mousePressed" ? 1 : 0,
    });
  }

  sendWheel(deltaY: number, deltaX = 0) {
    this.send({
      type: "input_mouse",
      eventType: "mouseWheel",
      x: 0,
      y: 0,
      deltaY,
      deltaX,
    });
  }

  sendKey(eventType: "keyDown" | "keyUp", key: string, code?: string) {
    this.send({
      type: "input_keyboard",
      eventType,
      key,
      code: code ?? key,
    });
  }

  disconnect() {
    this.closed = true;
    this.socket?.close();
    this.socket = null;
  }

  private send(payload: Record<string, unknown>) {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN) return;
    this.socket.send(JSON.stringify(payload));
  }
}

export function mapPreviewCoordinates(
  clientX: number,
  clientY: number,
  rect: DOMRect,
  metadata?: AgentBrowserStreamFrame["metadata"],
) {
  const width = metadata?.deviceWidth ?? rect.width;
  const height = metadata?.deviceHeight ?? rect.height;
  const scaleX = width / Math.max(rect.width, 1);
  const scaleY = height / Math.max(rect.height, 1);
  return {
    x: Math.round((clientX - rect.left) * scaleX),
    y: Math.round((clientY - rect.top) * scaleY),
  };
}