// Every outside link and version string on the site. Change them here.
const repo = "https://github.com/artificiadrian/rollport"

export const site = {
    version: "0.1.0",
    repo,
    author: "https://artificiadrian.com",
    releases: `${repo}/releases/latest`,
    /** The newest release's files: the release workflow names them without the version. */
    downloads: {
        mac: `${repo}/releases/latest/download/Rollport.dmg`,
        windows: `${repo}/releases/latest/download/Rollport-setup.exe`,
        linux: `${repo}/releases/latest/download/Rollport.AppImage`,
    },
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
