package de.wtzg.probewink

import android.app.Activity
import android.content.Context
import android.util.TypedValue
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import android.widget.ImageButton
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.Switch
import android.widget.TextView

/** Small view helpers so every screen uses the same cards, sections and rows. */
object UiKit {
    fun dp(context: Context, v: Int): Int =
        TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, v.toFloat(), context.resources.displayMetrics).toInt()

    /** Fill in the included top bar: title, optional back arrow and up to two actions. */
    fun topBar(
        a: Activity,
        title: String,
        back: Boolean,
        action: Pair<Int, () -> Unit>? = null,
        action2: Pair<Int, () -> Unit>? = null,
        actionDesc: String? = null,
        action2Desc: String? = null,
    ) {
        a.findViewById<TextView>(R.id.bar_title).text = title
        a.findViewById<ImageButton>(R.id.bar_back).apply {
            visibility = if (back) View.VISIBLE else View.GONE
            setOnClickListener { a.finish() }
        }
        a.findViewById<TextView>(R.id.bar_title).setPadding(dp(a, if (back) 4 else 12), 0, 0, 0)
        fun bind(id: Int, act: Pair<Int, () -> Unit>?, desc: String?) {
            a.findViewById<ImageButton>(id).apply {
                visibility = if (act == null) View.GONE else View.VISIBLE
                if (act != null) {
                    setImageResource(act.first)
                    contentDescription = desc
                    setOnClickListener { act.second() }
                }
            }
        }
        bind(R.id.bar_action, action, actionDesc)
        bind(R.id.bar_action2, action2, action2Desc)
    }

    /** Section title + a card; rows go into the returned card. */
    fun section(parent: LinearLayout, title: String?): LinearLayout {
        val ctx = parent.context
        if (title != null) {
            parent.addView(TextView(ctx).apply {
                setTextAppearance(R.style.T_Section)
                text = title
                setPadding(dp(ctx, 4), dp(ctx, 20), 0, dp(ctx, 8))
            })
        }
        val card = LinearLayout(ctx).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundResource(R.drawable.bg_card)
            clipToOutline = true
        }
        parent.addView(card, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        return card
    }

    /** A card with free text (explanations). */
    fun note(parent: LinearLayout, text: String, top: Int = 8) {
        val ctx = parent.context
        parent.addView(TextView(ctx).apply {
            setTextAppearance(R.style.T_Body)
            setTextColor(ctx.getColor(R.color.on_accent_soft))
            setBackgroundResource(R.drawable.bg_banner_info)
            setPadding(dp(ctx, 16), dp(ctx, 14), dp(ctx, 16), dp(ctx, 14))
            this.text = text
        }, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply {
            topMargin = dp(ctx, top)
        })
    }

    class Row(val view: View, val title: TextView, val summary: TextView, val end: FrameLayout) {
        fun summary(text: CharSequence?) {
            summary.text = text
            summary.visibility = if (text.isNullOrEmpty()) View.GONE else View.VISIBLE
        }
    }

    /** A list row: icon, title, summary, optional trailing view; dividers between rows. */
    fun row(
        card: LinearLayout,
        icon: Int,
        title: String,
        summary: CharSequence? = null,
        chevron: Boolean = false,
        onClick: (() -> Unit)? = null,
    ): Row {
        val ctx = card.context
        if (card.childCount > 0) {
            card.addView(View(ctx).apply { setBackgroundColor(ctx.getColor(R.color.outline)) },
                LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 1).apply { marginStart = dp(ctx, 56) })
        }
        val v = LayoutInflater.from(ctx).inflate(R.layout.row_item, card, false)
        card.addView(v)
        v.findViewById<ImageView>(R.id.row_icon).setImageResource(icon)
        val row = Row(v, v.findViewById(R.id.row_title), v.findViewById(R.id.row_summary), v.findViewById(R.id.row_end))
        row.title.text = title
        row.summary(summary)
        if (chevron) row.end.addView(ImageView(ctx).apply {
            setImageResource(R.drawable.ic_chevron)
            imageTintList = ctx.getColorStateList(R.color.muted)
        })
        if (onClick != null) v.setOnClickListener { onClick() } else v.isClickable = false
        return row
    }

    /** Row with a switch; tapping the row toggles it. */
    fun switchRow(card: LinearLayout, icon: Int, title: String, summary: String?, checked: Boolean, onChange: (Boolean) -> Unit): Row {
        val row = row(card, icon, title, summary)
        val sw = Switch(card.context).apply {
            isChecked = checked
            setOnCheckedChangeListener { _, on -> onChange(on) }
        }
        row.end.addView(sw)
        row.view.setOnClickListener { sw.toggle() }
        return row
    }

    /** Trailing status for progress rows: spinner, check or warning. */
    fun status(row: Row, kind: Coupler.Result?) {
        val ctx = row.view.context
        row.end.removeAllViews()
        val v: View = when (kind) {
            Coupler.Result.RUNNING -> ProgressBar(ctx).apply { isIndeterminate = true }
            Coupler.Result.OK -> ImageView(ctx).apply { setImageResource(R.drawable.ic_check); imageTintList = ctx.getColorStateList(R.color.ok) }
            Coupler.Result.FAILED -> ImageView(ctx).apply { setImageResource(R.drawable.ic_warning); imageTintList = ctx.getColorStateList(R.color.danger) }
            Coupler.Result.SKIPPED -> ImageView(ctx).apply { setImageResource(R.drawable.ic_chevron); imageTintList = ctx.getColorStateList(R.color.muted) }
            null -> return
        }
        row.end.addView(v, FrameLayout.LayoutParams(dp(ctx, 24), dp(ctx, 24)))
    }
}
