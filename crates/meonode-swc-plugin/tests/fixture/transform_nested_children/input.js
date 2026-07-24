import { Div, P } from "@meonode/ui";
Div({
    padding: "20px",
    onClick: h,
    children: [
        Div({
            color: "red"
        }),
        P("x", {
            color: "blue"
        })
    ]
});
