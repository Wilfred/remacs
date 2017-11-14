;;; remacs-helpers.el -- tools for Remacs development
;;; Commentary:

;; This is a collection of tools to help developers working with Remacs source.

(require 'dash)
(require 's)

;;; Code:

(defun remacs-helpers/ignored-type-part-p (input)
  "Predicate to indicate if INPUT is part of a C type ignored in Rust."
  (string= input "struct"))

(defun remacs-helpers/make-rust-args-from-C-worker (input)
  "Transform C function arguments INPUT into Rust style arguments.

For example, convert this:
int *foo, struct Bar b

into this:
foo: *mut int, b: Bar"
  (->> input
       (s-split ",")
       (-map #'s-trim)
       (--map (-remove 'remacs-helpers/ignored-type-part-p (s-split " " it)))
       (-map (-lambda ((type name))
               (if (s-starts-with-p "*" name)
                   (format "%s: *mut %s" (s-chop-prefix "*" name) type)
                 (format "%s: %s" name type))))
       (s-join ", ")))

(defun remacs-helpers/make-rust-args-from-C (string &optional from to)
  "Transform provided STRING or region indicated by FROM and TO into Rust style arguments."
  (interactive
   (if (use-region-p)
       (list nil (region-beginning) (region-end))
     (let ((bds (bounds-of-thing-at-point 'paragraph)) )
       (list nil (car bds) (cdr bds)) ) ) )

  (let* ((input (or string (buffer-substring-no-properties from to)))
         (output (remacs-helpers/make-rust-args-from-C-worker input)))
    (if string
        output
      (save-excursion
        (delete-region from to)
        (goto-char from)
        (insert output) )) ) )

(provide 'remacs-helpers)

;;; remacs-helpers.el ends here
