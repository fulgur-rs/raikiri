[Exposed=Window]
interface Node {
    readonly attribute unsigned short nodeType;
    readonly attribute Node? parentNode;
    boolean isSameNode(Node? otherNode);
};

[Exposed=Window]
interface Element : Node {
    DOMString? getAttribute(DOMString qualifiedName);
    undefined setAttribute(DOMString qualifiedName, DOMString value);
    undefined removeAttribute(DOMString qualifiedName);
};
