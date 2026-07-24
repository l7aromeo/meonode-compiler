import { Div } from "@meonode/ui";
Div({
    ref: myRef,
    css: {
        color: "red"
    },
    padding: "1px",
    key: "k1",
    as: "section",
    theme: myTheme,
    children: [
        "hi"
    ]
});
