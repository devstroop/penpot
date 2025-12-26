;; Rust Real-time Sync Integration
;; ================================
;; WebSocket-based real-time collaboration using Rust.

(ns app.rust-services.realtime
  "Real-time sync using Rust WebSocket service"
  (:require
   [app.common.logging :as log]
   [app.rust-services.client :as rust]))

;; ---------------------------------------------------------------------------
;; WebSocket URL Generation
;; ---------------------------------------------------------------------------

(defn get-ws-url
  "Get WebSocket URL for a file's real-time sync room.
   This URL should be provided to frontend clients."
  [file-id]
  (rust/get-realtime-ws-url file-id))

(defn get-client-config
  "Get configuration for frontend WebSocket client.
   Returns a map with connection details."
  [file-id user-id]
  {:ws-url   (get-ws-url file-id)
   :file-id  file-id
   :user-id  user-id
   :protocol "penpot-realtime-v1"})

;; ---------------------------------------------------------------------------
;; Message Types
;; ---------------------------------------------------------------------------

(def message-types
  "Supported real-time message types"
  {:join         "join"
   :leave        "leave"
   :cursor       "cursor"
   :selection    "selection"
   :shape-update "shape-update"
   :shape-create "shape-create"
   :shape-delete "shape-delete"})

(defn make-join-message
  "Create a join message for entering a room"
  [user-id user-name]
  {:type    (:join message-types)
   :user_id user-id
   :user_name user-name})

(defn make-cursor-message
  "Create a cursor position update message"
  [user-id x y page-id]
  {:type    (:cursor message-types)
   :user_id user-id
   :x       x
   :y       y
   :page_id page-id})

(defn make-selection-message
  "Create a selection update message"
  [user-id shape-ids]
  {:type      (:selection message-types)
   :user_id   user-id
   :shape_ids shape-ids})

(defn make-shape-update-message
  "Create a shape update message"
  [user-id shape-id changes]
  {:type     (:shape-update message-types)
   :user_id  user-id
   :shape_id shape-id
   :changes  changes})

;; ---------------------------------------------------------------------------
;; Service Info
;; ---------------------------------------------------------------------------

(defn get-service-stats
  "Get statistics from the real-time sync service.
   Returns promise with active rooms and connection counts."
  []
  (rust/check-service-health :realtime-sync))
