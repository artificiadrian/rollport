// The help pages and their sections, in order: the navigation lists them, and
// each section takes its heading from here by id. The ids are stable, so the
// app can link to them.
export const help = [
    { href: "/help", label: "Overview", sections: [] },
    {
        href: "/help/getting-started",
        label: "Get started",
        // Steps for one system only carry it: the sidebar shows them for that
        // system, as the page does.
        sections: [
            {
                id: "apple-devices",
                title: "Install Apple Devices",
                os: "windows",
            },
            { id: "usbmuxd", title: "Install usbmuxd", os: "linux" },
            { id: "install", title: "Download and install" },
            { id: "connect", title: "Connect your iPhone" },
            { id: "folder", title: "Choose a folder" },
            { id: "import", title: "Import" },
        ],
    },
    {
        href: "/help/using",
        label: "Using Rollport",
        sections: [
            { id: "options", title: "Choose what to import" },
            { id: "file-names", title: "How files are named" },
            { id: "copying", title: "During an import" },
            { id: "again", title: "After an import, and the next one" },
            { id: "folders", title: "Folders and iPhones" },
            { id: "keys", title: "Keyboard shortcuts" },
            { id: "remembered", title: "What Rollport remembers" },
        ],
    },
    {
        href: "/help/troubleshooting",
        label: "Troubleshooting",
        sections: [
            {
                id: "mac-cannot-open",
                title: "The Mac will not open Rollport",
            },
            {
                id: "windows-protected",
                title: "Windows says it protected your PC",
            },
            {
                id: "windows-apple-devices",
                title: "Windows does not find the iPhone",
            },
            { id: "linux-usbmuxd", title: "Linux does not find the iPhone" },
            {
                id: "nothing-happens",
                title: "Nothing happens when I connect the iPhone",
            },
            { id: "trust-refused", title: "Trust was refused" },
            {
                id: "cannot-connect",
                title: "Cannot connect to the iPhone",
            },
            {
                id: "camera-roll",
                title: "Cannot read the camera roll",
            },
            { id: "folder-unusable", title: "Cannot use this folder" },
            { id: "not-enough-space", title: "Not enough space" },
            {
                id: "not-copied",
                title: "Some files could not be copied",
            },
            {
                id: "folder-busy",
                title: "Another import is writing into this folder",
            },
            { id: "network-drive", title: "The folder is on a network drive" },
            { id: "linux-blank", title: "The window is empty on Linux" },
            {
                id: "windows-heic",
                title: "Windows cannot open the photos or videos",
            },
            { id: "forget", title: "Make Rollport forget its folders" },
            { id: "something-else", title: "Something else" },
        ],
    },
] as const satisfies readonly {
    href: string
    label: string
    sections: readonly {
        id: string
        title: string
        os?: "windows" | "linux"
    }[]
}[]

export type SectionId = (typeof help)[number]["sections"][number]["id"]

export function sectionTitle(id: SectionId): string {
    for (const page of help)
        for (const section of page.sections)
            if (section.id === id) return section.title
    throw new Error(`no help section ${id}`)
}
