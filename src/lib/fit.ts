import type { Attachment } from "svelte/attachments"

/// Shows the first of `texts` that fits the element, longest first; the last
/// one shows even when it does not, and the element's truncate cuts it.
/// The element must clip (truncate), so a text too long shows as overflow.
export function fitText(texts: string[]): Attachment<HTMLElement> {
    return (node) => {
        const pick = () => {
            for (const text of texts) {
                node.textContent = text
                if (node.scrollWidth <= node.clientWidth) return
            }
        }
        pick()
        // A wider or narrower pill, or a font that loads late.
        const observer = new ResizeObserver(pick)
        observer.observe(node.parentElement ?? node)
        return () => observer.disconnect()
    }
}
