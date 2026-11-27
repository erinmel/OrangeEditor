# Call an HTTP API

This guide sends a request from a graph and handles the three things that can come
back: a success status, an error status, and a request that never completed. The
[HTTP with failure handling](../examples/#http-with-failure-handling) example is the
finished graph.

## Create the connection

Every request goes through a connection, which holds the address and credentials so
the graph never contains them.

1. Open **Connections** and add an **HTTP** connection.
2. Set **Base URL** to the service, such as `https://api.example.com`. Only `http://`
   and `https://` addresses without user info are accepted.
3. Set a timeout. Requests that take longer fail instead of hanging the flow.
4. For an API key, enter it in the secret field. It is stored in the operating
   system's credential store; the graph keeps only a `credential://` reference.

The first time a graph uses a connection that reaches outside the computer, the editor
asks you to approve it for that graph.

## Send the request

1. Place a trigger, such as **On manual**, and an **HTTP request** node.
2. In **Properties**, choose the connection.
3. Set **method** (such as `GET` or `POST`) and **path**, which starts with `/` and is
   relative to the base URL.
4. For a body, set **content** and **content_type**. Add headers as text, one
   `Name: value` per line, or as a map in **header_fields**. Add query parameters in
   **query**.
5. Wire the trigger's **then** to the request's **in**.

Redirects are not followed, so a request only ever reaches the host you configured.

## Handle the outcome

The request continues on one of two routes:

- **then**: the server answered. Every status, including `404` or `500`, arrives
  here. Read **status** for the number, **success** for whether it was `2xx`,
  **response** for the body and **response_headers** for the headers.
- **failed**: no answer arrived, such as a refused connection or a timeout. Read
  **error** for what happened.

A common shape is a [**Branch**](../nodes/core/branch/1/#port-condition) on **success** after **then**, with one action for an
accepted status and another for a rejected one, plus a separate action on **failed**.

## Retry once after a short wait

Some failures are temporary. To retry once, wire **failed** to a **Wait** of, for
example, 250 ms, then to a second **HTTP request** with the same settings. Handle
that one's **failed** route as final. The
[bounded retry example](../examples/) shows the whole graph.

Retry only requests that are safe to repeat. A repeated `POST` can create a record
twice.

## See also

- [HTTP request node reference](../nodes/org.orange.net/http-request/2/)
- [Data wires and execution wires](../data-and-execution/)
