import Testing
@testable import VzEdge

@Test func mergeKeepsOneGlobalBlockAndConcatenatesSites() {
    let neti = """
    {
    \tauto_https off
    \thttp_port 18080
    \thttps_port 18443
    \tdefault_bind 127.0.0.1
    }

    https://main-node.svc.neti.vz.test {
    \treverse_proxy 10.90.0.2:18080
    }
    """
    let store = """
    {
    \tauto_https off
    \thttp_port 18080
    \thttps_port 18443
    \tdefault_bind 127.0.0.1
    }

    https://store.svc.iq-store.vz.test {
    \treverse_proxy 10.92.0.2:8080
    }
    """
    let merged = CaddyfileMerge.merge([neti, store])
    #expect(merged.contains("https://main-node.svc.neti.vz.test"))
    #expect(merged.contains("reverse_proxy 10.90.0.2:18080"))
    #expect(merged.contains("https://store.svc.iq-store.vz.test"))
    #expect(merged.contains("reverse_proxy 10.92.0.2:8080"))
    let globals = merged.components(separatedBy: "auto_https off")
    #expect(globals.count == 2)
}

@Test func mergeReturnsEmptyForEmptyInput() {
    #expect(CaddyfileMerge.merge([]) == "")
    #expect(CaddyfileMerge.merge(["", "  "]) == "")
}

@Test func mergeConcatenatesFilesWithoutGlobalBlock() {
    let merged = CaddyfileMerge.merge([
        "https://a.example {\n\treverse_proxy 127.0.0.1:1\n}",
        "https://b.example {\n\treverse_proxy 127.0.0.1:2\n}",
    ])
    #expect(merged.contains("https://a.example"))
    #expect(merged.contains("https://b.example"))
    #expect(!merged.contains("auto_https"))
}

@Test func mergeKeepsFirstGlobalWhenPortsDiffer() {
    let first = """
    {
    \tauto_https off
    \thttp_port 18080
    \thttps_port 18443
    }

    https://a.example {
    \treverse_proxy 10.1.0.2:80
    }
    """
    let second = """
    {
    \tauto_https off
    \thttp_port 9443
    \thttps_port 9444
    }

    https://b.example {
    \treverse_proxy 10.2.0.2:80
    }
    """
    let merged = CaddyfileMerge.merge([first, second])
    #expect(merged.contains("http_port 18080"))
    #expect(merged.contains("https_port 18443"))
    #expect(!merged.contains("http_port 9443"))
    #expect(!merged.contains("https_port 9444"))
    #expect(merged.contains("https://a.example"))
    #expect(merged.contains("https://b.example"))
    #expect(merged.components(separatedBy: "auto_https off").count == 2)
}

@Test func mergeTakesFirstAvailableGlobalWhenLeadingFileHasNone() {
    let sitesOnly = "https://a.example {\n\treverse_proxy 10.1.0.2:80\n}"
    let withGlobal = """
    {
    \tauto_https off
    \thttp_port 18080
    }

    https://b.example {
    \treverse_proxy 10.2.0.2:80
    }
    """
    let merged = CaddyfileMerge.merge([sitesOnly, withGlobal])
    #expect(merged.contains("auto_https off"))
    #expect(merged.contains("http_port 18080"))
    #expect(merged.contains("https://a.example"))
    #expect(merged.contains("https://b.example"))
    #expect(merged.components(separatedBy: "auto_https off").count == 2)
}

@Test func mergeKeepsBareGlobalBlock() {
    let merged = CaddyfileMerge.merge(["{\n\tauto_https off\n}"])
    #expect(merged == "{\n\tauto_https off\n}\n")
}
