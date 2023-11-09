/*
 * Copyright (C) 2023  Elliot Xu
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 */

package com.elliot00.liushu.input

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.elliot00.liushu.input.keyboard.KeyCode
import com.elliot00.liushu.input.keyboard.KeyboardLayout
import com.elliot00.liushu.service.ImeWeakReference
import com.elliot00.liushu.service.LiushuInputMethodServiceImpl
import com.elliot00.liushu.service.UselessLiushuInputMethodService
import com.elliot00.liushu.uniffi.Candidate
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update

class InputViewModel(
    private val ime: LiushuInputMethodServiceImpl = ImeWeakReference.get()
        ?: UselessLiushuInputMethodService()
) : ViewModel() {
    private var _isAsciiMode = MutableStateFlow(false)
    val isAsciiMode: StateFlow<Boolean> = _isAsciiMode.asStateFlow()
    private var _isCapital = MutableStateFlow(false)

    private var _input = MutableStateFlow("")
    private var _inputBuffer = MutableStateFlow("")
    val input =
        combine(_input, _inputBuffer) { a, b -> a + if (b.isNotEmpty()) " $b" else "" }.stateIn(
            viewModelScope, SharingStarted.WhileSubscribed(), ""
        )
    private var _candidates = MutableStateFlow<List<Candidate>>(emptyList())
    val candidates: StateFlow<List<Candidate>> = _candidates.asStateFlow()

    private var _keyboardLayout = MutableStateFlow(KeyboardLayout.QWERTY)
    val keyboardLayout = _keyboardLayout.asStateFlow()

    fun handleKey(keyCode: KeyCode) {
        when (keyCode) {
            is KeyCode.Alpha -> {
                handleValidAlphaKey(keyCode.code)
            }

            is KeyCode.RawText -> {
                ime.commitText(keyCode.text)
            }

            is KeyCode.AsciiModeSwitch -> {
                _isAsciiMode.update { !it }
            }

            is KeyCode.Enter -> {
                if (_input.value.isNotEmpty()) {
                    ime.commitText(_input.value + _inputBuffer.value)
                    _input.value = ""
                    _inputBuffer.value = ""
                } else {
                    ime.handleEnter()
                }
            }

            is KeyCode.Delete -> {
                if (_inputBuffer.value.isNotEmpty()) {
                    _inputBuffer.update { it.dropLast(1) }
                    return
                }
                if (_input.value.isNotEmpty()) {
                    _input.update { it.dropLast(1) }
                    _candidates.value = ime.search(_input.value)
                } else {
                    ime.handleDelete()
                }
            }

            is KeyCode.Shift -> {
                if (_input.value.isEmpty()) {
                    _isCapital.value = true
                }
            }

            is KeyCode.Comma -> {
                ime.commitText("，")
            }

            is KeyCode.Space -> {
                ime.commitText("　")
            }

            is KeyCode.Period -> {
                ime.commitText("。")
            }

            is KeyCode.Symbols -> {
                _keyboardLayout.value = KeyboardLayout.SYMBOLS
            }

            is KeyCode.Emoji -> {
                _keyboardLayout.value = KeyboardLayout.EMOJI
            }

            is KeyCode.Abc -> {
                _keyboardLayout.value = KeyboardLayout.QWERTY
            }

            else -> {}
        }
    }

    fun commitCandidate(candidate: Candidate) {
        _input.value = ""
        ime.commitText(candidate.text).also { _candidates.value = emptyList() }

        while (_inputBuffer.value.isNotEmpty()) {
            val potentialInput = _input.value + _inputBuffer.value.substring(0, 1)
            val potentialCandidates = ime.search(potentialInput)
            if (potentialCandidates.isEmpty()) {
                break
            }

            _input.value = potentialInput
            _inputBuffer.update { it.drop(1) }
            _candidates.value = potentialCandidates
        }
    }

    private fun handleValidAlphaKey(code: String) {
        if (_isAsciiMode.value) {
            ime.commitText(code)
            return
        }

        if (_isCapital.value) {
            ime.commitText(code.uppercase())
            _isCapital.value = false
            return
        }

        val potentialInput = _input.value + code
        val potentialCandidates = ime.search(potentialInput)
        if (potentialCandidates.isNotEmpty()) {
            _input.value = potentialInput
            _candidates.value = potentialCandidates
        } else {
            _inputBuffer.value += code
        }
    }
}