(() => {
    let error = null;
    try { element.setAttribute("data-surrogate", "\ud800"); }
    catch (e) { error = e.name; }
    return JSON.stringify({ roundtrip: element.getAttribute("data-surrogate") === "\ud800", error });
})();
