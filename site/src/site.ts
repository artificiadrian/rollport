// Every outside link and version string on the site. Change them here.
const repo = "https://github.com/artificiadrian/rollport"

export const site = {
    version: "0.1.0",
    repo,
    author: "https://artificiadrian.com",
    /** Every release, for the version history. */
    releases: `${repo}/releases`,
    appleDevices: "https://apps.microsoft.com/detail/9np83lwlpz9k",
    /** What Windows needs to open HEIC photos: HEVC is inside them too. */
    heif: "https://apps.microsoft.com/detail/9pmmsr1cgpwg",
    hevc: "https://apps.microsoft.com/detail/9nmzlz57r3t7",
    /** Where the site is served (GitHub Pages): astro.config.ts reads both. */
    origin: "https://artificiadrian.github.io",
    base: "/rollport",
}

/** What Rollport does: the home page and the share image say it. */
export const sentence =
    "Copy new photos and videos from your iPhone to a folder on your computer, over USB. Nothing on the phone is deleted."

/** A path on the site ("/help"), with the base it is served under. */
export const url = (path: string) => `${site.base}${path}`

export type Os = "mac" | "windows" | "linux"

/**
 * Every file a release has, in the order each system shows them; the first of
 * a system is its main one. The release workflow names them without the
 * version, so latest/download always gives the newest.
 */
export const files = [
    {
        os: "mac",
        name: "Rollport.dmg",
        format: ".dmg",
        detail: "Apple silicon and Intel",
    },
    {
        os: "windows",
        name: "Rollport-setup.exe",
        format: "Installer",
        detail: "Adds it to the Start menu",
        then: "Run it and follow its steps.",
    },
    {
        os: "windows",
        name: "Rollport.exe",
        format: "Portable",
        detail: "Runs from any folder",
        then: "Open Rollport.exe from where you saved it.",
    },
    {
        os: "linux",
        name: "Rollport.AppImage",
        format: "AppImage",
        detail: "Any distribution",
        command: "chmod +x Rollport.AppImage",
        then: "Then open it.",
    },
    {
        os: "linux",
        name: "Rollport.deb",
        format: ".deb",
        detail: "Debian, Ubuntu",
        command: "sudo apt install ./Rollport.deb",
    },
    {
        os: "linux",
        name: "Rollport.rpm",
        format: ".rpm",
        detail: "Fedora",
        command: "sudo dnf install ./Rollport.rpm",
    },
] as const satisfies readonly {
    os: Os
    name: string
    format: string
    detail?: string
    /** How to install it, on Get started: a command to type, then a note. */
    command?: string
    then?: string
}[]

export type ReleaseFile = (typeof files)[number]

/** Where a release file downloads from. */
export const fileUrl = (name: string) =>
    `${site.repo}/releases/latest/download/${name}`

/** A system's main file: the one its button gives. */
export const mainFile = (os: Os) => files.find((file) => file.os === os)!

/** Every download button opens Get started, which starts the file and shows its steps. */
export const downloadPage = (name: string) =>
    `${url("/help/getting-started")}?file=${encodeURIComponent(name)}`
