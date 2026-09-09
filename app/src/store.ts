// 轻量全局状态：当前页、底部状态栏统计、轻提示
import { reactive } from "vue"
import { api } from "./api"

export type PageId =
  | "clean"
  | "large"
  | "dupes"
  | "space"
  | "uninstall"
  | "recovery"
  | "settings"

interface Toast {
  id: number
  text: string
  kind: "info" | "success" | "error"
}

export const store = reactive({
  page: "clean" as PageId,
  stats: { freedTotalBytes: 0, recoveryBytes: 0 },
  toasts: [] as Toast[],

  async refreshStats() {
    try {
      this.stats = await api.getStats()
    } catch {
      /* 后端未就绪时静默 */
    }
  },

  go(page: PageId) {
    this.page = page
  },

  toast(text: string, kind: Toast["kind"] = "info") {
    const id = Date.now() + Math.random()
    this.toasts.push({ id, text, kind })
    setTimeout(() => {
      this.toasts = this.toasts.filter((t) => t.id !== id)
    }, 3200)
  },
})

export function applyTheme(theme: string) {
  const dark =
    theme === "dark" ||
    (theme === "system" &&
      window.matchMedia("(prefers-color-scheme: dark)").matches)
  document.documentElement.dataset.theme = dark ? "dark" : "light"
}
