;; Rust Render Service Integration
;; ================================
;; Server-side rendering using Rust for exports and thumbnails.

(ns app.rust-services.render
  "Rendering operations using Rust microservice"
  (:require
   [app.common.logging :as log]
   [app.rust-services.client :as rust]
   [promesa.core :as p]))

;; ---------------------------------------------------------------------------
;; Export Operations
;; ---------------------------------------------------------------------------

(defn export-page
  "Export a page to the specified format using Rust renderer.
   
   Options:
   - :file-id  - UUID of the file
   - :page-id  - UUID of the page
   - :format   - Export format (:png, :svg, :pdf)
   - :scale    - Scale factor (default 1.0)
   - :shapes   - Optional list of shape IDs to export (nil = all)"
  [{:keys [file-id page-id format scale shapes] :as opts}]
  (log/debug :msg "Rust export requested"
             :file-id file-id
             :page-id page-id
             :format format)
  (rust/render-page-rust opts))

(defn export-shapes
  "Export specific shapes to the specified format."
  [file-id page-id shape-ids format]
  (export-page {:file-id file-id
                :page-id page-id
                :format  format
                :shapes  shape-ids}))

;; ---------------------------------------------------------------------------
;; Thumbnail Generation
;; ---------------------------------------------------------------------------

(defn generate-thumbnail
  "Generate a thumbnail for a page using Rust renderer."
  [file-id page-id]
  (log/debug :msg "Rust thumbnail requested"
             :file-id file-id
             :page-id page-id)
  (rust/generate-thumbnail-rust {:file-id file-id :page-id page-id}))

(defn generate-file-thumbnails
  "Generate thumbnails for all pages in a file."
  [file-id page-ids]
  (p/all
   (map #(generate-thumbnail file-id %) page-ids)))

;; ---------------------------------------------------------------------------
;; Batch Operations
;; ---------------------------------------------------------------------------

(defn batch-export
  "Export multiple pages/formats in a single batch.
   
   Items should be a sequence of maps with :file-id, :page-id, :format keys."
  [items]
  (p/all (map export-page items)))

;; ---------------------------------------------------------------------------
;; Format Support
;; ---------------------------------------------------------------------------

(def supported-formats
  "Formats supported by the Rust render service"
  #{:png :svg :pdf})

(defn format-supported?
  "Check if a format is supported by the Rust renderer"
  [format]
  (contains? supported-formats (keyword format)))
