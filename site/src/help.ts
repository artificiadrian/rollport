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
            { id: "again", title: "The next import" },
            { id: "folders", title: "More than one folder" },
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
            {
                id: "windows-heic",
                title: "Windows cannot open the photos or videos",
            },
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
