import Film from "@lucide/svelte/icons/film"
import ImageIcon from "@lucide/svelte/icons/image"
import MonitorPlay from "@lucide/svelte/icons/monitor-play"
import Scan from "@lucide/svelte/icons/scan"
import SquarePen from "@lucide/svelte/icons/square-pen"
import Target from "@lucide/svelte/icons/target"
import Video from "@lucide/svelte/icons/video"
import type { Kind } from "$lib/flow"

/// What each kind of file looks like: in Media types, and on a print with
/// no photo of its own.
export const ICONS = {
    photos: ImageIcon,
    livePhotos: Target,
    liveVideos: Film,
    videos: Video,
    screenshots: Scan,
    screenRecordings: MonitorPlay,
    edits: SquarePen,
} satisfies Record<Kind | "edits", unknown>
